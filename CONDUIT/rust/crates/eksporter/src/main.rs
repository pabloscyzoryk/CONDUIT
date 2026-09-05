
use anyhow::{anyhow, Context, Result};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use conduit_telegram::client::{ClientConfig, TelegramClient};
use conduit_telegram::dialogs::{self, DialogEntry, DialogKind};
use conduit_telegram::photos::PhotoCache;
use conduit_telegram::session::FileSession;

// ═══════════════════════════════════════════════════════════════════════
//  KSZTAŁT PLIKU WYJŚCIOWEGO
// ═══════════════════════════════════════════════════════════════════════

/// Nagłówek `result.json` — dokładnie cztery pola, w tej kolejności co Desktop.
#[derive(Serialize)]
struct Eksport {
    name: String,
    /// `private_channel`, `public_channel`, `private_group`, `personal_chat`
    #[serde(rename = "type")]
    typ: String,
    /// identyfikator BEZ przedrostka `-100` — Desktop zapisuje go dodatnio
    id: i64,
    messages: Vec<Wiadomosc>,
}

#[derive(Serialize)]
struct Wiadomosc {
    id: i64,
    #[serde(rename = "type")]
    typ: &'static str,
    /// czas LOKALNY, tak jak zapisuje Telegram Desktop
    date: String,
    date_unixtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    edited: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edited_unixtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<i64>,
    text: String,
    text_entities: Vec<Encja>,
}

#[derive(Serialize)]
struct Encja {
    #[serde(rename = "type")]
    typ: &'static str,
    text: String,
}

fn czas_lokalny(sekundy: i64) -> String {
    match Local.timestamp_opt(sekundy, 0).single() {
        Some(t) => t.format("%Y-%m-%dT%H:%M:%S").to_string(),
        // Przesunięcie strefy potrafi dać godzinę nieistniejącą albo podwójną.
        // Wtedy zapisujemy UTC — lepiej mieć znacznik o godzinę obok niż
        // urwać eksport, bo `date_unixtime` i tak niesie prawdę.
        None => Utc
            .timestamp_opt(sekundy, 0)
            .single()
            .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string())
            .unwrap_or_default(),
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  STAN SERWERA
// ═══════════════════════════════════════════════════════════════════════

struct Stan {
    klient: grammers_client::Client,
    czaty: Vec<DialogEntry>,
    katalog: PathBuf,
    /// Miniatury awatarów. Ten sam magazyn co w panelu bota — pobiera
    /// obrazek RAZ i wraca do Telegrama dopiero, gdy zmieni się `photo_id`.
    zdjecia: PhotoCache,
}

#[derive(Serialize)]
struct CzatWidok {
    chat_id: i64,
    nazwa: String,
    rodzaj: &'static str,
    forum: bool,
    /// czy warto w ogóle pytać o miniaturę (czat bez zdjęcia dostaje literkę)
    ma_zdjecie: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Zlecenie {
    chat_id: i64,
    /// `0` = bez ograniczenia (cała historia)
    #[serde(default)]
    limit: usize,
}

#[derive(Serialize)]
struct Wynik {
    plik: String,
    wiadomosci: usize,
    od: String,
    do_: String,
}

// ═══════════════════════════════════════════════════════════════════════
//  POBIERANIE
// ═══════════════════════════════════════════════════════════════════════

async fn pobierz(stan: &Stan, chat_id: i64, limit: usize) -> Result<Wynik> {
    let wpis = stan
        .czaty
        .iter()
        .find(|c| c.chat_id == chat_id)
        .ok_or_else(|| anyhow!("nie znam czatu {chat_id} — odśwież listę"))?;

    let mut iter = stan.klient.iter_messages(wpis.peer.clone());
    let mut zebrane: Vec<Wiadomosc> = Vec::new();

    while let Some(m) = iter.next().await.context("pobieranie historii")? {
        let ts = m.date().timestamp();
        let tekst = m.text().to_string();
        let encje = if tekst.is_empty() {
            Vec::new()
        } else {
            vec![Encja {
                typ: "plain",
                text: tekst.clone(),
            }]
        };
        // Desktop zapisuje `from_id` jako `channel<id>` / `user<id>`, gdzie
        // `<id>` jest DODATNI i bez przedrostka `-100`. Konwencja Bot API daje
        // `-1000000000042` dla przykładowego kanału, więc bierzemy wartość bezwzględną
        // i ucinamy przedrostek — tak powstaje `channel000000000042`.
        //
        // GDY `sender()` ZWRACA `None`. Przy wiadomości opublikowanej przez
        // sam kanał nadawcy nie ma — podpisuje się kanał. Nadawcą jest wtedy
        // czat, z którego pobieramy; pozostawienie `from_id: null` psuje
        // zgodność identyfikacji wiadomości.
        let zrodlo = m
            .sender()
            .map(|p| p.id().bot_api_dialog_id_unchecked())
            .unwrap_or(wpis.chat_id);
        let pref = if zrodlo < 0 { "channel" } else { "user" };
        let nadawca_id = Some(format!("{pref}{}", zrodlo.abs() % 1_000_000_000_000));
        let nadawca = Some(wpis.name.clone());
        zebrane.push(Wiadomosc {
            id: m.id() as i64,
            typ: "message",
            date: czas_lokalny(ts),
            date_unixtime: ts.to_string(),
            edited: m.edit_date().map(|d| czas_lokalny(d.timestamp())),
            edited_unixtime: m.edit_date().map(|d| d.timestamp().to_string()),
            from: nadawca,
            from_id: nadawca_id,
            reply_to_message_id: m.reply_to_message_id().map(|x| x as i64),
            text: tekst,
            text_entities: encje,
        });
        if limit > 0 && zebrane.len() >= limit {
            break;
        }
    }

    // Telegram oddaje historię od NAJNOWSZEJ; Desktop zapisuje rosnąco po id.
    zebrane.sort_by_key(|w| w.id);

    let od = zebrane.first().map(|w| w.date.clone()).unwrap_or_default();
    let do_ = zebrane.last().map(|w| w.date.clone()).unwrap_or_default();

    let typ = match (&wpis.kind, wpis.username.is_some()) {
        (DialogKind::Channel, true) => "public_channel",
        (DialogKind::Channel, false) => "private_channel",
        (DialogKind::Group, true) => "public_supergroup",
        (DialogKind::Group, false) => "private_group",
        (DialogKind::User, _) => "personal_chat",
    };

    let dok = Eksport {
        name: wpis.name.clone(),
        typ: typ.to_string(),
        // Desktop zapisuje identyfikator bez przedrostka `-100`.
        id: wpis.chat_id.abs() % 1_000_000_000_000,
        messages: zebrane,
    };

    let stempel = Local::now().format("%Y-%m-%d_%H%M%S");
    let katalog = stan.katalog.join(format!("ChatExport_{stempel}"));
    std::fs::create_dir_all(&katalog).context("tworzenie katalogu eksportu")?;
    let plik = katalog.join("result.json");
    let tresc = serde_json::to_string(&dok).context("składanie JSON-a")?;
    std::fs::write(&plik, tresc).with_context(|| format!("zapis {}", plik.display()))?;

    Ok(Wynik {
        plik: plik.display().to_string(),
        wiadomosci: dok.messages.len(),
        od,
        do_,
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  TRASY
// ═══════════════════════════════════════════════════════════════════════

async fn strona() -> Html<&'static str> {
    Html(STRONA)
}

async fn czaty(State(s): State<Arc<Stan>>) -> Json<Vec<CzatWidok>> {
    Json(
        s.czaty
            .iter()
            .map(|c| CzatWidok {
                chat_id: c.chat_id,
                nazwa: c.name.clone(),
                rodzaj: match c.kind {
                    DialogKind::Channel => "kanał",
                    DialogKind::Group => "grupa",
                    DialogKind::User => "osoba",
                },
                forum: c.is_forum,
                ma_zdjecie: c.photo_id.is_some(),
            })
            .collect(),
    )
}

/// Miniatura awatara. `404` znaczy „ten czat nie ma zdjęcia" i jest
/// normalną odpowiedzią — interfejs rysuje wtedy literkę.
async fn foto(
    State(s): State<Arc<Stan>>,
    axum::extract::Path(chat_id): axum::extract::Path<i64>,
) -> Response {
    let Some(w) = s.czaty.iter().find(|c| c.chat_id == chat_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match s
        .zdjecia
        .ensure(&s.klient, chat_id, w.peer.clone(), w.photo_id)
        .await
    {
        Ok(Some(p)) => match std::fs::read(&p) {
            Ok(b) => ([(header::CONTENT_TYPE, "image/jpeg")], b).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn eksportuj(State(s): State<Arc<Stan>>, Json(z): Json<Zlecenie>) -> Response {
    match pobierz(&s, z.chat_id, z.limit).await {
        Ok(w) => Json(w).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({ "blad": e.to_string() }).to_string(),
        )
            .into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  START
// ═══════════════════════════════════════════════════════════════════════

/// Szuka `secrets.json` tam, gdzie naprawdę bywa: obok exe, w katalogu
/// roboczym, w paczkach. Pierwszy trafiony wygrywa.
fn znajdz_secrets() -> Option<PathBuf> {
    let mut kand: Vec<PathBuf> = vec![
        PathBuf::from("secrets.json"),
        PathBuf::from("VPSREADY/secrets.json"),
        PathBuf::from("../VPSREADY/secrets.json"),
        PathBuf::from("PACKAGE/secrets.json"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            kand.insert(0, d.join("secrets.json"));
        }
    }
    kand.into_iter().find(|p| p.exists())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let sciezka = znajdz_secrets().ok_or_else(|| {
        anyhow!(
            "nie znalazłem secrets.json — uruchom eksport.exe obok conduit.exe \
             albo w katalogu paczki (VPSREADY / PACKAGE / RELEASE)"
        )
    })?;
    println!("poświadczenia: {}", sciezka.display());

    let tresc = std::fs::read_to_string(&sciezka)
        .with_context(|| format!("czytanie {}", sciezka.display()))?;
    let dok: conduit_server::secrets::SecretsDoc =
        serde_json::from_str(&tresc).context("secrets.json ma nieoczekiwany kształt")?;

    if !dok.telegram.has_credentials() {
        return Err(anyhow!("secrets.json nie ma pary apiId/apiHash"));
    }
    if !dok.telegram.has_session() {
        return Err(anyhow!(
            "secrets.json nie ma sesji — zaloguj się najpierw w panelu CONDUIT"
        ));
    }

    // WŁASNY plik sesji, nigdy `conduit.session` — patrz nagłówek modułu.
    let plik_sesji = sciezka
        .parent()
        .unwrap_or(Path::new("."))
        .join("eksport.session");
    FileSession::restore_string_session(&plik_sesji, dok.telegram.session_string.as_str())
        .map_err(|e| anyhow!("nie mogę odtworzyć sesji: {e}"))?;

    let klient = TelegramClient::connect(ClientConfig {
        api_id: dok.telegram.api_id,
        api_hash: dok.telegram.api_hash.as_str().to_string(),
        session_path: plik_sesji,
        // Eksporter nie słucha aktualizacji — czyta historię na żądanie.
        catch_up: false,
        ignore_outgoing: false,
        queue_limit: 8,
    })
    .await
    .context("logowanie do Telegrama")?;

    let lista = dialogs::list_dialogs(klient.raw(), 500)
        .await
        .map_err(|e| anyhow!("nie mogę pobrać listy czatów: {e}"))?;
    println!("czatów widocznych: {}", lista.len());

    let katalog = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let zdjecia = PhotoCache::new(&katalog.join("miniatury"));
    let stan = Arc::new(Stan {
        klient: klient.raw().clone(),
        czaty: lista,
        katalog,
        zdjecia,
    });

    let app = Router::new()
        .route("/", get(strona))
        .route("/api/czaty", get(czaty))
        .route("/api/foto/{chat_id}", get(foto))
        .route("/api/eksport", post(eksportuj))
        .with_state(stan);

    let adres = SocketAddr::from(([127, 0, 0, 1], 8766));
    let sluchacz = tokio::net::TcpListener::bind(adres)
        .await
        .with_context(|| format!("nie mogę zająć {adres} — czy eksport już działa?"))?;
    let url = format!("http://{adres}");
    println!("EKSPORTER: {url}");
    let _ = open::that(&url);
    axum::serve(sluchacz, app).await.context("serwer")?;
    Ok(())
}

const STRONA: &str = r#"<!doctype html>
<html lang="pl"><head><meta charset="utf-8">
<title>CONDUIT — eksporter historii</title>
<style>
 :root{--tlo:#0f1115;--pole:#171a21;--ramka:#2a2f3a;--tekst:#e6e8ec;--przygas:#9aa3b2;--akcent:#4f9cf9}
 *{box-sizing:border-box} body{margin:0;background:var(--tlo);color:var(--tekst);
   font:15px/1.5 system-ui,Segoe UI,sans-serif;padding:28px}
 h1{font-size:20px;margin:0 0 4px} p.pod{color:var(--przygas);margin:0 0 22px}
 input,button{font:inherit;color:inherit;background:var(--pole);
   border:1px solid var(--ramka);border-radius:8px;padding:9px 11px}
 input{width:100%} label{display:block;margin:14px 0 5px;color:var(--przygas);font-size:13px}
 button{background:var(--akcent);border-color:var(--akcent);color:#05070c;font-weight:600;
   cursor:pointer;margin-top:18px;padding:10px 18px}
 button[disabled]{opacity:.5;cursor:default}
 .karta{max-width:660px;background:var(--pole);border:1px solid var(--ramka);
   border-radius:12px;padding:20px}
 .lista{margin-top:8px;max-height:340px;overflow:auto;border:1px solid var(--ramka);
   border-radius:8px;background:#12151b}
 .rzad{display:flex;align-items:center;gap:11px;padding:8px 11px;cursor:pointer;
   border-bottom:1px solid #1e222b}
 .rzad:last-child{border-bottom:none}
 .rzad:hover{background:#1a1e26}
 .rzad.wybrany{background:#1d2c44;box-shadow:inset 3px 0 0 var(--akcent)}
 .awatar{width:34px;height:34px;border-radius:50%;flex:0 0 34px;object-fit:cover;
   background:#2a2f3a;display:grid;place-items:center;font-weight:600;color:var(--przygas);
   font-size:14px;overflow:hidden}
 .nazwa{flex:1;min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
 .meta{color:var(--przygas);font-size:12px;white-space:nowrap}
 .wynik{margin-top:18px;padding:14px;border-radius:8px;border:1px solid var(--ramka);
   background:#12151b;white-space:pre-wrap;word-break:break-all}
 .blad{border-color:#7f1d1d;background:#1b1113}
 .pusto{padding:14px;color:var(--przygas)}
</style></head><body>
<div class="karta">
  <h1>Eksporter historii</h1>
  <p class="pod">Plik wychodzi w tym samym formacie, co eksport z Telegram Desktop.</p>

  <label for="szukaj">Czat — szukaj po nazwie</label>
  <input id="szukaj" placeholder="wpisz fragment nazwy, np. synergy" autocomplete="off">
  <div class="lista" id="lista"><div class="pusto">wczytuję…</div></div>

  <label for="limit">Ile ostatnich wiadomości (0 = cała historia)</label>
  <input id="limit" type="number" min="0" step="100" value="0">

  <button id="idz" disabled>Pobierz eksport</button>
  <div class="wynik" id="wynik" style="display:none"></div>
</div>
<script>
let czaty=[], wybrany=null;
const szukaj=document.getElementById('szukaj'),lista=document.getElementById('lista'),
      wyn=document.getElementById('wynik'),btn=document.getElementById('idz');

fetch('/api/czaty').then(r=>r.json()).then(d=>{czaty=d; rysuj();});
szukaj.addEventListener('input',rysuj);

function rysuj(){
  const q=szukaj.value.trim().toLowerCase();
  const w=czaty.filter(c=>!q||c.nazwa.toLowerCase().includes(q));
  lista.innerHTML='';
  if(!w.length){lista.innerHTML='<div class="pusto">nic nie pasuje</div>';return;}
  w.forEach(c=>{
    const r=document.createElement('div');
    r.className='rzad'+(wybrany===c.chat_id?' wybrany':'');
    // Awatar pobieramy DOPIERO dla czatów, które go mają — inaczej każda
    // pozycja listy generowałaby zapytanie kończące się na 404.
    const a=document.createElement(c.ma_zdjecie?'img':'div');
    a.className='awatar';
    if(c.ma_zdjecie){a.src='/api/foto/'+c.chat_id; a.loading='lazy';
      a.onerror=()=>{const z=document.createElement('div');z.className='awatar';
        z.textContent=(c.nazwa[0]||'?').toUpperCase();a.replaceWith(z);};}
    else a.textContent=(c.nazwa[0]||'?').toUpperCase();
    const n=document.createElement('div'); n.className='nazwa'; n.textContent=c.nazwa;
    const m=document.createElement('div'); m.className='meta';
    m.textContent=c.rodzaj+(c.forum?' · tematy':'');
    r.append(a,n,m);
    r.addEventListener('click',()=>{wybrany=c.chat_id; btn.disabled=false; rysuj();});
    lista.appendChild(r);
  });
}

btn.addEventListener('click',()=>{
  if(wybrany===null) return;
  btn.disabled=true; wyn.style.display='block'; wyn.className='wynik';
  wyn.textContent='pobieram… przy pełnej historii to potrafi potrwać kilka minut';
  fetch('/api/eksport',{method:'POST',headers:{'Content-Type':'application/json'},
    body:JSON.stringify({chatId:wybrany,limit:Number(document.getElementById('limit').value)||0})})
   .then(r=>r.json()).then(d=>{
     btn.disabled=false;
     if(d.blad){wyn.className='wynik blad'; wyn.textContent='BŁĄD: '+d.blad; return;}
     wyn.textContent='gotowe — '+d.wiadomosci+' wiadomości
'+d.od+'  →  '+d.do_+'

'+d.plik;
   }).catch(e=>{btn.disabled=false; wyn.className='wynik blad'; wyn.textContent='BŁĄD: '+e;});
});
</script></body></html>"#;
