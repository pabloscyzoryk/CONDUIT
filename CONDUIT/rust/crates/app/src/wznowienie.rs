
use conduit_core::types::{
    Basket, BasketEvent, BasketState, PendingOrder, Position, Px, Side, SourceKey, Ticket, Ts,
};
use conduit_server::store::Workspace;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Wersja formatu. Zrzut o innej wersji jest odrzucany w całości — lepiej
/// odtworzyć koszyki z samego konta niż wczytać pola, które znaczą co innego.
pub const WERSJA: u32 = 1;

/// Nazwa pliku obok `settings.json`. Świadomie w katalogu roboczym, a nie
/// w `backup_memory/`: tamten katalog trzyma migawkę INTERFEJSU (`ui::Basket`,
/// bez drabinki siatki i bez `msg_id`) i rotuje co 15 sekund. To są dwie różne
/// rzeczy o podobnej nazwie i mieszanie ich kosztowałoby dzień.
pub const PLIK: &str = "koszyki.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zrzut {
    pub wersja: u32,
    /// znacznik czasu zapisu (ms epoki)
    pub zapisano: i64,
    /// numer rachunku, na którym te koszyki powstały
    pub login: i64,
    pub magic: i64,
    pub symbol: String,
    pub next_basket_id: u32,
    pub koszyki: Vec<Basket>,
}

pub fn sciezka(ws: &Workspace) -> PathBuf {
    ws.root.join(PLIK)
}

/// Zapisuje koszyki silnika. Zapis jest atomowy (`write_json_atomic`), więc
/// wyłączenie prądu w trakcie zapisu nie zostawia pliku w połowie.
pub fn zapisz(
    ws: &Workspace,
    koszyki: &[Basket],
    next_basket_id: u32,
    login: i64,
    magic: i64,
    symbol: &str,
    teraz: i64,
) -> anyhow::Result<()> {
    let z = Zrzut {
        wersja: WERSJA,
        zapisano: teraz,
        login,
        magic,
        symbol: symbol.to_string(),
        next_basket_id,
        koszyki: koszyki.to_vec(),
    };
    conduit_server::store::write_json_atomic(&sciezka(ws), &z)
}

/// Wczytuje zrzut. `None`, gdy pliku nie ma albo jest nieczytelny — brak
/// zrzutu NIE jest błędem, tylko uboższym wznowieniem.
pub fn wczytaj(ws: &Workspace) -> Option<Zrzut> {
    let p = sciezka(ws);
    let tekst = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str::<Zrzut>(&tekst).ok()
}

/// Follow-terminal recovery is isolated by the entire account identity.
/// The legacy dump deliberately remains unchanged for the opt-out path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopedZrzut {
    pub wersja: u32,
    pub login: i64,
    pub server: String,
    pub trade_mode: i32,
    pub magic: i64,
    pub symbol: String,
    pub zrzut: Zrzut,
}

fn poprawna_tozsamosc(login: i64, server: &str, trade_mode: i32, symbol: &str) -> bool {
    login > 0 && !server.trim().is_empty() && (0..=2).contains(&trade_mode)
        && !symbol.trim().is_empty()
}

/// Hashing a length-delimited tuple prevents server/symbol strings from becoming
/// paths. Full metadata is checked again on read, not just the file name.
pub fn sciezka_scoped(
    ws: &Workspace, login: i64, server: &str, trade_mode: i32, magic: i64, symbol: &str,
) -> PathBuf {
    let key = serde_json::to_vec(&(login, server, trade_mode, magic, symbol))
        .expect("a primitive account tuple is serializable");
    ws.root.join("account_state").join(format!("{:x}", Sha256::digest(key))).join(PLIK)
}

pub fn zapisz_scoped(
    ws: &Workspace, koszyki: &[Basket], next_basket_id: u32, login: i64,
    magic: i64, symbol: &str, teraz: i64, server: &str, trade_mode: i32,
) -> anyhow::Result<()> {
    anyhow::ensure!(poprawna_tozsamosc(login, server, trade_mode, symbol),
        "refusing to persist baskets without a complete account identity");
    let envelope = ScopedZrzut {
        wersja: 1, login, server: server.to_owned(), trade_mode, magic,
        symbol: symbol.to_owned(),
        zrzut: Zrzut {
            wersja: WERSJA, zapisano: teraz, login, magic, symbol: symbol.to_owned(),
            next_basket_id, koszyki: koszyki.to_vec(),
        },
    };
    conduit_server::store::write_json_atomic(
        &sciezka_scoped(ws, login, server, trade_mode, magic, symbol), &envelope)
}

/// Never fall back to root/koszyki.json: an unscoped login alone does not prove
/// either the broker or DEMO/REAL identity. The caller may reconcile only with
/// positions actually returned by the newly pinned broker connection.
pub fn wczytaj_scoped(
    ws: &Workspace, login: i64, server: &str, trade_mode: i32, magic: i64, symbol: &str,
) -> Option<Zrzut> {
    if !poprawna_tozsamosc(login, server, trade_mode, symbol) { return None; }
    let p = sciezka_scoped(ws, login, server, trade_mode, magic, symbol);
    let e: ScopedZrzut = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
    if e.wersja != 1 || e.login != login || e.server != server
        || e.trade_mode != trade_mode || e.magic != magic || e.symbol != symbol
        || e.zrzut.wersja != WERSJA || e.zrzut.login != login
        || e.zrzut.magic != magic || e.zrzut.symbol != symbol {
        return None;
    }
    Some(e.zrzut)
}

/// Wynik odtworzenia — same liczby, żeby dało się je wpisać do dziennika
/// i porównać z `ReconcileReport` mostu.
#[derive(Debug, Clone, Default)]
pub struct Wynik {
    /// koszyki do przekazania do `Engine::adopt_baskets`
    pub koszyki: Vec<Basket>,
    /// ile z nich przyszło z pełną treścią ze zrzutu
    pub z_zrzutu: usize,
    /// ile trzeba było zlepić z samych zleceń u brokera
    pub z_konta: usize,
    /// koszyki ze zrzutu, po których u brokera nie ma już śladu
    pub wygasle: usize,
    /// SIEROTY WŁAŚCIWE: nasz `magic`, ale komentarza NIE DA SIĘ odczytać,
    /// więc nie wiadomo, do czego pozycja należała. To jest awaria i to
    /// zasługuje na maila.
    ///
    /// Najczęstsza przyczyna: własny komentarz dłuższy niż 29 znaków —
    /// `order_send` zwraca wtedy `None` z błędem −2 i zlecenie znika bez
    /// śladu, a to, co przeszło, ma komentarz obcięty nie do odczytania.
    pub sieroty: usize,
    pub reczne: usize,
    /// szczeble siatki zablokowane przed odtworzeniem (patrz nagłówek pliku)
    pub zablokowane_szczeble: usize,
    /// powód odrzucenia zrzutu, jeśli został odrzucony
    pub zrzut_odrzucony: Option<String>,
    /// Ile pozycji i zleceń przypisano do koszyka PO TICKECIE ZE ZRZUTU,
    /// bo komentarz był nieczytelny albo obcięty. Liczba > 0 znaczy, że
    /// bez tej ścieżki tyle rzeczy zostałoby sierotami.
    pub po_tickecie: usize,
    /// czy treść koszyków w ogóle przyszła ze zrzutu — bez tego dziennik
    /// pisał „źródło treści: plik koszyki.json" także wtedy, gdy pliku
    /// nie było wcale, a koszyki zlepiono z samych zleceń
    pub zrzut_uzyty: bool,
}

impl Wynik {
    /// Jedno zdanie do dziennika i do maila.
    pub fn opis(&self) -> String {
        let mut s = format!(
            "koszyki odtworzone: {} (ze zrzutu {}, z samego konta {}) · wygasłe {} · \
             SIEROTY (komentarz nieczytelny) {} · ręczne bilety z panelu (to normalne) {} · \
             szczebli zablokowanych przed powtórnym wejściem {} ·              przypisanych PO TICKECIE ze zrzutu {}",
            self.koszyki.len(),
            self.z_zrzutu,
            self.z_konta,
            self.wygasle,
            self.sieroty,
            self.reczne,
            self.zablokowane_szczeble,
            self.po_tickecie
        );
        if let Some(p) = &self.zrzut_odrzucony {
            s.push_str(&format!("\nZRZUT ODRZUCONY: {p}"));
        }
        s
    }
}

/// Składa listę koszyków do przejęcia przez silnik.
///
/// `pozycje` i `zlecenia` to **nasze** pozycje i zlecenia po rekoncyliacji
/// mostu (most odsiewa cudzy `magic` sam) — czyli dokładnie to, co zwraca
/// `Broker::positions()` / `Broker::pendings()` zaraz po `connect`.
///
/// `tag` to znacznik komentarza (`comment_mode` z panelu) — po nim poznajemy,
/// czy komentarz jest NASZ i czytelny. Bez tego nie da się odróżnić ręcznego
/// biletu z panelu od prawdziwej sieroty.
pub fn odtworz(
    zrzut: Option<Zrzut>,
    pozycje: &[Position],
    zlecenia: &[PendingOrder],
    login: i64,
    magic: i64,
    symbol: &str,
    tag: &str,
) -> Wynik {
    let mut w = Wynik::default();

    // ---- 1. czy zrzut w ogóle dotyczy TEGO rachunku ----
    let zrzut = match zrzut {
        None => None,
        Some(z) if z.wersja != WERSJA => {
            w.zrzut_odrzucony = Some(format!(
                "format {} zamiast {WERSJA} — plik z innej wersji programu",
                z.wersja
            ));
            None
        }
        // Rachunek, magic i symbol muszą się zgadzać. Wczytanie koszyków
        // z INNEGO konta byłoby najgorszym z możliwych błędów: silnik
        // zarządzałby pozycjami, których tam nie ma, i liczyłby etapy celów
        // z cudzego sygnału.
        Some(z) if login != 0 && z.login != 0 && z.login != login => {
            w.zrzut_odrzucony = Some(format!(
                "zrzut z rachunku {}, a terminal jest zalogowany na {login}",
                z.login
            ));
            None
        }
        Some(z) if z.magic != magic => {
            w.zrzut_odrzucony = Some(format!(
                "zrzut dla magic {}, a w ustawieniach jest {magic}",
                z.magic
            ));
            None
        }
        Some(z) if !z.symbol.eq_ignore_ascii_case(symbol) => {
            w.zrzut_odrzucony = Some(format!(
                "zrzut dla symbolu {}, a bot gra na {symbol}",
                z.symbol
            ));
            None
        }
        Some(z) => Some(z),
    };
    w.zrzut_uzyty = zrzut.is_some();

    let mut z_ticketu: std::collections::HashMap<Ticket, u32> = std::collections::HashMap::new();
    if let Some(z) = &zrzut {
        for bk in &z.koszyki {
            if !bk.alive() {
                continue;
            }
            for t in bk.tickets.iter().chain(bk.pendings.iter()) {
                z_ticketu.insert(*t, bk.id);
            }
        }
    }

    let mut id_z_konta: Vec<u32> = Vec::new();
    let mut odzyskane_po_tickecie = 0usize;
    for p in pozycje {
        match p.basket.or_else(|| z_ticketu.get(&p.ticket).copied()) {
            Some(b) => {
                if p.basket.is_none() {
                    odzyskane_po_tickecie += 1;
                }
                if !id_z_konta.contains(&b) {
                    id_z_konta.push(b);
                }
            }
            None => policz_bez_koszyka(&p.comment, tag, &mut w),
        }
    }
    for o in zlecenia {
        match o.basket.or_else(|| z_ticketu.get(&o.ticket).copied()) {
            Some(b) => {
                if o.basket.is_none() {
                    odzyskane_po_tickecie += 1;
                }
                if !id_z_konta.contains(&b) {
                    id_z_konta.push(b);
                }
            }
            None => policz_bez_koszyka(&o.comment, tag, &mut w),
        }
    }
    id_z_konta.sort_unstable();
    w.po_tickecie = odzyskane_po_tickecie;

    // ---- 3. koszyki ze zrzutu ----
    let mut gotowe: Vec<Basket> = Vec::new();
    if let Some(z) = &zrzut {
        for bk in &z.koszyki {
            if !bk.alive() {
                continue;
            }
            let ma_cokolwiek = id_z_konta.contains(&bk.id);
            if !ma_cokolwiek {
                // Koszyk, po którym u brokera nie ma ANI JEDNEJ pozycji i ANI
                // JEDNEGO zlecenia, jest skończony — wszystko, czym można było
                // zarządzać, zamknęło się w czasie, gdy bot nie działał.
                // Wskrzeszanie go dałoby pusty wiersz w panelu i koszyk,
                // któremu reguła przezbrojenia mogłaby dostawić zlecenia.
                w.wygasle += 1;
                continue;
            }
            let mut bk = bk.clone();
            dopasuj_do_konta(&mut bk, pozycje, zlecenia, &mut w.zablokowane_szczeble);
            gotowe.push(bk);
            w.z_zrzutu += 1;
        }
    }

    // ---- 4. koszyki, których w zrzucie nie ma, a na koncie są ----
    for id in &id_z_konta {
        if gotowe.iter().any(|b| b.id == *id) {
            continue;
        }
        if let Some(bk) = z_samego_konta(*id, pozycje, zlecenia) {
            gotowe.push(bk);
            w.z_konta += 1;
        }
    }

    gotowe.sort_by_key(|b| b.id);
    w.koszyki = gotowe;
    w
}

/// Pozycja bez koszyka: RĘCZNY BILET czy SIEROTA?
///
/// Rozstrzyga komentarz. `decode` zwraca strukturę tylko wtedy, gdy komentarz
/// zaczyna się naszym znacznikiem i ma poprawny kształt `<tag><koszyk>.<poziom>`.
/// Pusty koszyk w czytelnym komentarzu (`CDx.0`) znaczy „ta pozycja NIGDY nie
/// miała koszyka" — tak wygląda bilet otwarty przyciskiem w panelu. Komentarz,
/// którego nie da się odczytać, znaczy „nie wiemy, czym to było" — i dopiero to
/// jest awaria.
fn policz_bez_koszyka(komentarz: &str, tag: &str, w: &mut Wynik) {
    if conduit_mt5::comment::decode(tag, komentarz).is_some() {
        w.reczne += 1;
    } else {
        w.sieroty += 1;
    }
}

/// Uzgadnia koszyk ze zrzutu z faktami u brokera.
fn dopasuj_do_konta(
    bk: &mut Basket,
    pozycje: &[Position],
    zlecenia: &[PendingOrder],
    zablokowane: &mut usize,
) {
    // Przynależność rozstrzyga komentarz ALBO lista ticketów ze zrzutu.
    // Ten sam powód co przy budowie `id_z_konta`: komentarz bywa obcięty
    // na 29 znakach, a zrzut jest naszym własnym zapisem. Bez drugiego
    // warunku koszyk wracał tu z PUSTĄ listą pozycji — czyli formalnie
    // odtworzony, a faktycznie niczym nie zarządzający.
    let ze_zrzutu: std::collections::HashSet<Ticket> = bk
        .tickets
        .iter()
        .chain(bk.pendings.iter())
        .copied()
        .collect();

    bk.tickets = pozycje
        .iter()
        .filter(|p| p.basket == Some(bk.id) || ze_zrzutu.contains(&p.ticket))
        .map(|p| p.ticket)
        .collect();
    bk.pendings = zlecenia
        .iter()
        .filter(|o| o.basket == Some(bk.id) || ze_zrzutu.contains(&o.ticket))
        .map(|o| o.ticket)
        .collect();

    if !bk.tickets.is_empty() {
        bk.had_positions = true;
        if bk.state == BasketState::Armed {
            bk.state = BasketState::Working;
        }
    }

    // Zamknięcie furtki do powtórnego wejścia — uzasadnienie w nagłówku pliku.
    for gl in bk.levels.iter_mut() {
        if gl.filled {
            continue;
        }
        let stoi = zlecenia.iter().any(|o| {
            (o.basket == Some(bk.id) || bk.pendings.contains(&o.ticket)) && o.level == gl.level
        });
        if !stoi {
            gl.filled = true;
            *zablokowane += 1;
        }
    }
}

/// Ostatnia deska ratunku: koszyk zlepiony z samych zleceń i pozycji.
///
/// Odtwarzamy tylko to, co broker naprawdę trzyma: stronę, strefę (z cen
/// wejścia), SL i cele (z pól zlecenia). `levels` zostaje PUSTE — nie znamy
/// planu siatki, a pusty plan jest jedyną wartością, przy której żadna reguła
/// nie spróbuje dostawić zlecenia.
fn z_samego_konta(id: u32, pozycje: &[Position], zlecenia: &[PendingOrder]) -> Option<Basket> {
    let poz: Vec<&Position> = pozycje.iter().filter(|p| p.basket == Some(id)).collect();
    let zle: Vec<&PendingOrder> = zlecenia.iter().filter(|o| o.basket == Some(id)).collect();
    if poz.is_empty() && zle.is_empty() {
        return None;
    }

    let side: Side = poz
        .first()
        .map(|p| p.side)
        .or_else(|| zle.first().map(|o| o.kind.side()))?;

    let mut ceny: Vec<Px> = poz.iter().map(|p| p.open_price).collect();
    ceny.extend(zle.iter().map(|o| o.price));
    let lo = ceny.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = ceny.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // SL: bierzemy najdalszy od rynku, czyli najbezpieczniejszy z widocznych.
    let sl = poz
        .iter()
        .filter_map(|p| p.sl)
        .chain(zle.iter().filter_map(|o| o.sl))
        .fold(None::<Px>, |a, s| {
            Some(match (a, side) {
                (None, _) => s,
                (Some(x), Side::Buy) => x.min(s),
                (Some(x), Side::Sell) => x.max(s),
            })
        });

    // Cele: wszystkie różne TP widoczne na zleceniach, ułożone po kolei
    // w stronę wejścia. To nie jest pełna drabinka z sygnału (runner nie ma
    // TP i nie zostawia śladu), ale wystarcza, żeby etapy celów liczyły się
    // dalej od miejsca, w którym stanęły.
    let mut tps: Vec<Px> = poz
        .iter()
        .filter_map(|p| p.tp)
        .chain(zle.iter().filter_map(|o| o.tp))
        .collect();
    tps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    tps.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    if side == Side::Sell {
        tps.reverse();
    }

    let ts: Ts = poz
        .iter()
        .map(|p| p.open_ts)
        .chain(zle.iter().map(|o| o.placed_ts))
        .min()
        .unwrap_or(0);

    Some(Basket {
        //  Przesuniecie warstw z tresci sygnalu — po restarcie nie mamy
        //  tekstu, wiec koszyk odtworzony gra geometria z ustawien.
        warstwy_offset: None,
        id,
        source: SourceKey::new(0, None),
        source_name: "ODTWORZONY Z KONTA".to_string(),
        // Ujemny i pochodny od numeru koszyka: przestrzeń identyfikatorów
        // Telegrama jest dodatnia, więc kolizja jest niemożliwa, a wiązanie
        // odpowiedzi po `msg_id` po prostu nie znajdzie takiego koszyka —
        // i słusznie, bo nie wiemy, z której wiadomości powstał.
        msg_id: -(id as i64) - 1_000_000_000,
        msg_aliases: Vec::new(),
        persisted_done_actions: Vec::new(),
        pending_exit: None,
        pending_relot_review: Vec::new(),
        entry_edit_state: None,
        side,
        is_limit: !zle.is_empty(),
        // Odtwarzamy koszyk z SAMEGO STANU RACHUNKU, bez wiadomości źródłowej,
        // więc nie da się orzec, czy sygnał brzmiał „limit" czy „stop" — a
        // zlecenia oczekujące i tak już leżą u brokera po właściwej cenie.
        // `false` znaczy tu „nie wiemy", nie „na pewno limit"; pole steruje
        // wyłącznie SKŁADANIEM nowych zleceń, czego przy wznowieniu nie robimy.
        is_stop: false,
        // Uwolnienia regułą też nie odtworzymy — to fakt z historii decyzji,
        // a nie ze stanu pozycji. `false` jest tu bezpieczne: koszyk zostanie
        // potraktowany jak nieuwolniony, czyli podlega normalnemu zarządzaniu
        // zamiast zostać domknięty przez sweep zaraz po starcie.
        secured_by_rule: false,
        entry_lo: lo,
        entry_hi: hi,
        zone_lo: lo,
        zone_hi: hi,
        sl,
        tps,
        tp_stage: 0,
        // Po restarcie NIE WIEMY, ile celów rynek osiągnął bez nas — a zgadywać
        // nie wolno, bo ta liczba steruje kasowaniem siatki. Zero znaczy „licz
        // drogę od nowa", czyli najostrożniejszy z możliwych wyborów.
        plan_wykonany_do: 0,
        created_ts: ts,
        drop_po_ts: 0,
        state: if poz.is_empty() {
            BasketState::Armed
        } else {
            BasketState::Working
        },
        tickets: poz.iter().map(|p| p.ticket).collect(),
        pendings: zle.iter().map(|o| o.ticket).collect(),
        realized: 0.0,
        events: vec![BasketEvent {
            ts,
            text: format!(
                "koszyk odtworzony PO RESTARCIE z samego konta ({} pozycji, {} zleceń) — \
                 brak zrzutu koszyków, więc drabinka celów i plan siatki są niepełne",
                poz.len(),
                zle.len()
            ),
        }],
        levels: Vec::new(),
        // ZERO ZNACZY „NIE WIEMY", i to jest jedyna uczciwa wartość dla koszyka
        // odtworzonego z samego konta. Obie liczby silnik prowadzi sam:
        // `risk_initial_usd` wypełnia przy rozstawianiu siatki, gdy jest ≤ 0
        // (engine.rs:2083), a `peak_pl_usd` narasta od zera przy każdym ticku
        // (engine.rs:626). Wpisanie tu zgadywanej liczby zrobiłoby dwie szkody:
        // zablokowałoby wyliczenie ryzyka, a zawyżony szczyt natychmiast
        // wyprodukowałby `drawdown_from_peak` (engine.rs:591) z powietrza.
        peak_pl_usd: 0.0,
        risk_initial_usd: 0.0,
        // Pusty wektor, a NIE wypełniony zerami: `on_tick` sam go dociąga do
        // długości drabinki (`engine.rs:633`), a 0 znaczy „cel jeszcze nie
        // dotknięty". Wpisanie tu jakiejkolwiek chwili udawałoby, że
        // widzieliśmy dotknięcie celu, którego nikt nie widział — a na tej
        // liczbie stoją okna `tp_signal_max_lead_s` / `tp_signal_max_lag_s`.
        tp_touch_ts: Vec::new(),
        tp_touch_px: Vec::new(),
        // 0 = „nie dotknięty". Koszyk odtworzony z konta nie ma prawa twierdzić,
        // że widział dotknięcie stopa — nikt tego nie mierzył.
        sl_touch_ts: 0,
        sl_touch_px: 0.0,
        // 0 = „obowiązuje limit globalny". Filtr tempa ocenia koszyk raz, przy
        // trzeciej wypełnionej warstwie — a odtworzony koszyk ma tę chwilę już
        // za sobą i nie znamy jej. Wpisanie tu skróconego życia ukarałoby
        // koszyk za coś, czego nikt nie zmierzył; wpisanie długiego udawałoby,
        // że filtr go przepuścił. Globalny limit jest jedyną uczciwą opcją.
        age_limit_min: 0.0,
        // 0 = „nie jesteśmy pod wodą za naszej pamięci" — licznik ruszy przy
        // pierwszym ticku, na którym koszyk faktycznie będzie na minusie.
        adverse_since: 0,
        // 0 = „żaden cel nie był trafiony za naszej pamięci". Gdyby wpisać tu
        // chwilę restartu, `reenter_min_return_s` zablokowałoby dokładanie na
        // czas odstępu po zdarzeniu, które nie zaszło.
        last_tp_ts: 0,
        // Koszyk odtworzony z konta nie był oceniany przez filtr tempa
        // — nie wolno mu przypisać ani przelotu, ani zrobionej dokładki.
        tempo_fast: false,
        tempo_checked: false,
        pyramided: false,
        fast_addons: 0,
        last_addon_ts: 0,
        reentries: 0,
        last_entry_px: poz.iter().map(|p| p.open_price).next_back(),
        secured: false,
        // Nie mamy historii wiadomości, więc nie wolno odgadywać, że SPP
        // zakazało późniejszego rearmu. `false` zachowuje kontrakt sprzed osi.
        rearm_blocked_by_spp: false,
        secured_ts: 0,
        had_positions: !poz.is_empty(),
        tp_open: false,
        rearms: 0,
        last_rearm_ts: 0,
        wol_pierwotny: Vec::new(),
        zone_touched: !poz.is_empty(),
        // 0 = „kanał nigdy nie kazał postawić BE" — jedyna uczciwa wartość dla
        // koszyka odtworzonego z samego konta. Znacznik jest FAKTEM Z HISTORII
        // KOMUNIKATÓW, nie ze stanu pozycji: wpisanie tu chwili restartu
        // kazałoby osi `be_covers_late_fills` uznać wszystkie zastane pozycje
        // za powstałe PRZED komendą, której nikt nie widział.
        be_ts: 0,
        drop_armed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_core::types::{GridLevel, PendingKind};

    fn poz(ticket: u64, basket: Option<u32>, level: i32) -> Position {
        Position {
            ticket,
            side: Side::Buy,
            volume: 0.01,
            open_price: 4020.0,
            open_ts: 1_000,
            sl: Some(4008.0),
            tp: Some(4038.0),
            vsl: None,
            basket,
            level,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: false,
            comment: String::new(),
        }
    }

    fn zlec(ticket: u64, basket: Option<u32>, level: i32, price: Px) -> PendingOrder {
        PendingOrder {
            ticket,
            kind: PendingKind::BuyLimit,
            volume: 0.01,
            price,
            sl: Some(4008.0),
            tp: Some(4038.0),
            placed_ts: 2_000,
            basket,
            level,
            frozen: false,
            is_toucher: false,
            comment: String::new(),
            is_topup: false,
        }
    }

    fn koszyk(id: u32, levels: Vec<i32>) -> Basket {
        Basket {
            warstwy_offset: None,
            id,
            source: SourceKey::new(-100, None),
            source_name: "KANAŁ".into(),
            msg_id: 55,
            msg_aliases: vec![],
            persisted_done_actions: vec![],
            pending_exit: None,
            pending_relot_review: Vec::new(),
            entry_edit_state: None,
            side: Side::Buy,
            is_limit: true,
            entry_lo: 4020.0,
            entry_hi: 4025.0,
            zone_lo: 4020.0,
            zone_hi: 4025.0,
            sl: Some(4008.0),
            tps: vec![4038.0, 4048.0],
            tp_stage: 1,
            plan_wykonany_do: 0,
            created_ts: 500,
            state: BasketState::Armed,
            tickets: vec![],
            pendings: vec![],
            realized: 12.5,
            events: vec![],
            levels: levels
                .into_iter()
                .map(|l| GridLevel {
                    price: 4020.0 + l as f64,
                    base_units: 1,
                    volume: 0.01,
                    sl: Some(4008.0),
                    tp: Some(4038.0),
                    level: l,
                    is_toucher: false,
                    filled: false,
                    // 0 / false = „nie wypełniony i nie skasowany". Pomocnik
                    // testowy buduje szczebel ŚWIEŻO rozstawiony.
                    fill_ts: 0,
                    fill_px: 0.0,
                    cancelled: false,
                })
                .collect(),
            reentries: 0,
            last_entry_px: None,
            peak_pl_usd: 0.0,
            risk_initial_usd: 0.0,
            tp_touch_ts: Vec::new(),
            tp_touch_px: Vec::new(),
            sl_touch_ts: 0,
            sl_touch_px: 0.0,
            secured: false,
            rearm_blocked_by_spp: false,
            secured_ts: 0,
            had_positions: false,
            tp_open: false,
            rearms: 0,
            last_rearm_ts: 0,
            zone_touched: false,
            drop_armed: false,
            age_limit_min: 0.0,
            last_tp_ts: 0,
            tempo_fast: false,
            tempo_checked: false,
            pyramided: false,
            fast_addons: 0,
            last_addon_ts: 0,
            adverse_since: 0,
            is_stop: false,
            secured_by_rule: false,
            // Pola dodane później (okno łaski kasowania siatki i partiale od
            // wolumenu pierwotnego) — stan „nic się jeszcze nie wydarzyło".
            drop_po_ts: 0,
            wol_pierwotny: Vec::new(),
            // 0 = kanał nie kazał stawiać BE. Koszyk testowy nie ma historii
            // komunikatów, więc jedyna uczciwa wartość to „nigdy".
            be_ts: 0,
        }
    }

    fn zrzut(koszyki: Vec<Basket>) -> Zrzut {
        Zrzut {
            wersja: WERSJA,
            zapisano: 9_000,
            login: 10_000_001,
            magic: 770_077,
            symbol: "XAUUSD".into(),
            next_basket_id: 9,
            koszyki,
        }
    }

    /// SEDNO CAŁEGO PLIKU: pozycja z numerem koszyka w komentarzu nie może
    /// zostać po restarcie sierotą.
    #[test]
    fn koszyk_ze_zrzutu_wraca_z_pelna_drabinka() {
        let w = odtworz(
            Some(zrzut(vec![koszyk(1, vec![0, 1])])),
            &[poz(111, Some(1), 0)],
            &[zlec(222, Some(1), 1, 4020.0)],
            10_000_001,
            770_077,
            "XAUUSD",
            "CD",
        );
        assert_eq!(w.koszyki.len(), 1);
        assert_eq!(w.z_zrzutu, 1);
        assert_eq!(w.z_konta, 0);
        let b = &w.koszyki[0];
        assert_eq!(
            b.tps,
            vec![4038.0, 4048.0],
            "drabinka celów przeżywa restart"
        );
        assert_eq!(b.tp_stage, 1, "etap celów przeżywa restart");
        assert_eq!(
            b.msg_id, 55,
            "wiązanie odpowiedzi z kanału przeżywa restart"
        );
        assert_eq!(b.realized, 12.5);
        assert_eq!(b.tickets, vec![111]);
        assert_eq!(b.pendings, vec![222]);
        assert!(
            b.had_positions,
            "koszyk z otwartą pozycją nie jest już „czekającym sygnałem”"
        );
        assert_eq!(b.state, BasketState::Working);
    }

    #[test]
    fn pending_exit_przezywa_serde_i_odtworzenie_a_stary_zrzut_ma_none() {
        let b = koszyk(1, vec![0, 1]);
        let mut old = serde_json::to_value(&b).unwrap();
        old.as_object_mut().unwrap().remove("pending_exit");
        let restored_old: Basket = serde_json::from_value(old).unwrap();
        assert!(restored_old.pending_exit.is_none());

        let mut b = b;
        b.pending_exit = Some(conduit_core::types::PendingBasketExit {
            reason: conduit_core::types::CloseReason::Tp,
            last_attempt_ts: 8_000,
        });
        let saved = serde_json::to_value(zrzut(vec![b])).unwrap();
        let loaded: Zrzut = serde_json::from_value(saved).unwrap();
        let w = odtworz(
            Some(loaded),
            &[poz(111, Some(1), 0)],
            &[zlec(222, Some(1), 1, 4020.0)],
            10_000_001,
            770_077,
            "XAUUSD",
            "CD",
        );
        let intent = w.koszyki[0].pending_exit.as_ref().expect("exit survives restart");
        assert_eq!(intent.reason, conduit_core::types::CloseReason::Tp);
        assert_eq!(intent.last_attempt_ts, 8_000);
    }

    /// Szczebel bez żywego zlecenia nie ma prawa się odtworzyć — po restarcie
    /// nie wiemy, czy został skasowany, czy zrealizowany i zamknięty.
    #[test]
    fn szczebel_bez_zlecenia_jest_zablokowany() {
        let w = odtworz(
            Some(zrzut(vec![koszyk(1, vec![0, 1, 2])])),
            &[],
            &[zlec(222, Some(1), 1, 4020.0)],
            10_000_001,
            770_077,
            "XAUUSD",
            "CD",
        );
        let b = &w.koszyki[0];
        let po_poziomie = |l: i32| b.levels.iter().find(|g| g.level == l).unwrap().filled;
        assert!(po_poziomie(0), "szczebel 0 bez zlecenia — zablokowany");
        assert!(
            !po_poziomie(1),
            "szczebel 1 ma żywe zlecenie — zostaje otwarty"
        );
        assert!(po_poziomie(2));
        assert_eq!(w.zablokowane_szczeble, 2);
    }

    #[test]
    fn koszyk_bez_sladu_u_brokera_nie_zmartwychwstaje() {
        let w = odtworz(
            Some(zrzut(vec![koszyk(1, vec![0]), koszyk(2, vec![0])])),
            &[poz(111, Some(2), 0)],
            &[],
            10_000_001,
            770_077,
            "XAUUSD",
            "CD",
        );
        assert_eq!(w.koszyki.len(), 1);
        assert_eq!(w.koszyki[0].id, 2);
        assert_eq!(w.wygasle, 1);
    }

    /// Brak zrzutu to NIE jest powód, żeby zostawić pozycje bez opieki.
    #[test]
    fn bez_zrzutu_koszyk_powstaje_z_samego_konta() {
        let w = odtworz(
            None,
            &[poz(111, Some(7), 0)],
            &[zlec(222, Some(7), 1, 4015.0)],
            10_000_001,
            770_077,
            "XAUUSD",
            "CD",
        );
        assert_eq!(w.koszyki.len(), 1);
        assert_eq!(w.z_konta, 1);
        let b = &w.koszyki[0];
        assert_eq!(b.id, 7);
        assert_eq!(b.side, Side::Buy);
        assert_eq!(b.zone_lo, 4015.0, "strefa z najniższej ceny wejścia");
        assert_eq!(b.zone_hi, 4020.0);
        assert_eq!(b.sl, Some(4008.0));
        assert_eq!(b.tps, vec![4038.0]);
        assert!(
            b.levels.is_empty(),
            "bez planu siatki nic nie dostawi zleceń"
        );
        assert!(b.had_positions);
    }

    /// Zrzut z innego rachunku jest gorszy niż brak zrzutu.
    #[test]
    fn zrzut_z_innego_konta_jest_odrzucany() {
        let mut z = zrzut(vec![koszyk(1, vec![0])]);
        z.login = 11_111_111;
        let w = odtworz(
            Some(z),
            &[poz(111, Some(1), 0)],
            &[],
            10_000_001,
            770_077,
            "XAUUSD",
            "CD",
        );
        assert!(w.zrzut_odrzucony.is_some(), "{:?}", w.zrzut_odrzucony);
        assert!(
            !w.zrzut_uzyty,
            "odrzucony zrzut nie może się meldować jako źródło treści"
        );
        assert_eq!(w.z_zrzutu, 0);
        assert_eq!(w.z_konta, 1, "koszyk i tak wraca — tyle że uboższy");
    }

    #[test]
    fn zrzut_dla_innego_magica_i_symbolu_jest_odrzucany() {
        let mut z = zrzut(vec![koszyk(1, vec![0])]);
        z.magic = 202_406;
        let w = odtworz(Some(z), &[], &[], 10_000_001, 770_077, "XAUUSD", "CD");
        assert!(w.zrzut_odrzucony.as_deref().unwrap().contains("202406"));

        let mut z = zrzut(vec![koszyk(1, vec![0])]);
        z.symbol = "EURUSD".into();
        let w = odtworz(Some(z), &[], &[], 10_000_001, 770_077, "XAUUSD", "CD");
        assert!(w.zrzut_odrzucony.as_deref().unwrap().contains("EURUSD"));
    }

    /// Pozycja naszym magikiem, ale z komentarzem, z którego nie da się
    /// odczytać koszyka. Musi być POLICZONA — to jest liczba, po której widać,
    /// że coś jest nie tak z komentarzami.
    #[test]
    fn pozycje_bez_numeru_koszyka_sa_liczone() {
        let w = odtworz(
            None,
            &[poz(111, None, 0)],
            &[zlec(222, None, 0, 4020.0)],
            0,
            770_077,
            "XAUUSD",
            "CD",
        );
        assert_eq!(w.sieroty, 2, "pusty komentarz to sierota, nie ręczny bilet");
        assert_eq!(w.reczne, 0);
        assert!(w.koszyki.is_empty());
    }

    #[test]
    fn reczny_bilet_z_panelu_to_nie_sierota() {
        let mut reczna = poz(111, None, 0);
        reczna.comment = "CDx.0-panel".into();
        let mut sierota = poz(112, None, 0);
        sierota.comment = "TEST 2030-01-02 03:4".into(); // syntetyczny obcięty komentarz
        let mut reczne_zlec = zlec(222, None, 0, 4020.0);
        reczne_zlec.comment = "CDx.0-panel".into();

        let w = odtworz(
            None,
            &[reczna, sierota],
            &[reczne_zlec],
            0,
            770_077,
            "XAUUSD",
            "CD",
        );
        assert_eq!(
            w.reczne, 2,
            "dwa czytelne komentarze bez koszyka = ręczne bilety"
        );
        assert_eq!(w.sieroty, 1, "jeden nieczytelny = jedna prawdziwa sierota");
        assert!(
            w.opis().contains("SIEROTY (komentarz nieczytelny) 1"),
            "{}",
            w.opis()
        );
        assert!(
            w.opis().contains("ręczne bilety z panelu (to normalne) 2"),
            "{}",
            w.opis()
        );
    }

    /// Zmiana znacznika w ustawieniach nie może zamienić własnych pozycji
    /// w sieroty przy samym odczycie — ale MUSI to pokazać, a nie przemilczeć.
    #[test]
    fn inny_znacznik_robi_z_czytelnego_komentarza_sierote() {
        let mut p = poz(111, None, 0);
        p.comment = "CDx.0-panel".into();
        let w = odtworz(None, &[p], &[], 0, 770_077, "XAUUSD", "INNY");
        assert_eq!(w.sieroty, 1);
        assert_eq!(w.reczne, 0);
    }

    #[test]
    fn zapis_i_odczyt_daja_to_samo() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-wznow-{}-{}",
            std::process::id(),
            conduit_server::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::new(&dir);

        assert!(
            wczytaj(&ws).is_none(),
            "bez pliku ma być None, a nie panika"
        );
        zapisz(
            &ws,
            &[koszyk(3, vec![0, 1])],
            4,
            10_000_001,
            770_077,
            "XAUUSD",
            1234,
        )
        .unwrap();
        let z = wczytaj(&ws).expect("zrzut musi się wczytać");
        assert_eq!(z.wersja, WERSJA);
        assert_eq!(z.next_basket_id, 4);
        assert_eq!(z.koszyki.len(), 1);
        assert_eq!(z.koszyki[0].id, 3);
        assert_eq!(z.koszyki[0].levels.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scoped_test_workspace(label: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("conduit-scoped-{label}-{}-{}",
            std::process::id(), conduit_server::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        Workspace::new(dir)
    }

    #[test]
    fn scoped_account_switch_roundtrip_preserves_each_accounts_exit_intent() {
        let ws = scoped_test_workspace("switch");
        let mut a = koszyk(3, vec![0, 1]);
        a.pending_exit = Some(conduit_core::types::PendingBasketExit {
            reason: conduit_core::types::CloseReason::Tp, last_attempt_ts: 8_000,
        });
        zapisz_scoped(&ws, &[a], 4, 123, 77, "XAUUSD", 90, "Vantage-PUBLIC-DEMO", 0).unwrap();
        zapisz_scoped(&ws, &[koszyk(8, vec![2])], 9, 123, 77, "XAUUSD.s", 95, "PUPrime-PUBLIC-LIVE", 2).unwrap();
        let b = wczytaj_scoped(&ws, 123, "PUPrime-PUBLIC-LIVE", 2, 77, "XAUUSD.s").unwrap();
        assert_eq!(b.koszyki[0].id, 8);
        assert!(b.koszyki[0].pending_exit.is_none());
        let a = wczytaj_scoped(&ws, 123, "Vantage-PUBLIC-DEMO", 0, 77, "XAUUSD").unwrap();
        assert_eq!(a.next_basket_id, 4);
        assert_eq!(a.koszyki[0].id, 3);
        assert_eq!(a.koszyki[0].pending_exit.as_ref().unwrap().last_attempt_ts, 8_000);
        assert!(wczytaj(&ws).is_none(), "scoped writes cannot overwrite legacy state");
    }

    #[test]
    fn scoped_rejects_every_identity_dimension_and_never_reads_legacy() {
        let ws = scoped_test_workspace("identity");
        zapisz(&ws, &[koszyk(9, vec![0])], 10, 123, 77, "XAUUSD", 90).unwrap();
        assert!(wczytaj_scoped(&ws, 123, "broker", 0, 77, "XAUUSD").is_none());
        zapisz_scoped(&ws, &[koszyk(3, vec![0])], 4, 123, 77, "XAUUSD", 90, "broker", 0).unwrap();
        for (login, server, mode, magic, symbol) in [
            (124, "broker", 0, 77, "XAUUSD"), (123, "other", 0, 77, "XAUUSD"),
            (123, "broker", 2, 77, "XAUUSD"), (123, "broker", 0, 78, "XAUUSD"),
            (123, "broker", 0, 77, "XAUUSD.s"),
        ] {
            assert!(wczytaj_scoped(&ws, login, server, mode, magic, symbol).is_none());
        }
        assert_eq!(wczytaj(&ws).unwrap().koszyki[0].id, 9);
    }

    #[test]
    fn scoped_rejects_forged_metadata_versions_and_broken_files() {
        let ws = scoped_test_workspace("metadata");
        zapisz_scoped(&ws, &[], 4, 123, 77, "XAUUSD", 90, "broker", 0).unwrap();
        let path = sciezka_scoped(&ws, 123, "broker", 0, 77, "XAUUSD");
        let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for (pointer, value) in [
            ("/wersja", serde_json::json!(2)), ("/login", serde_json::json!(124)),
            ("/server", serde_json::json!("other")), ("/trade_mode", serde_json::json!(2)),
            ("/magic", serde_json::json!(78)), ("/symbol", serde_json::json!("XAUUSD.s")),
            ("/zrzut/wersja", serde_json::json!(2)),
            ("/zrzut/login", serde_json::json!(124)), ("/zrzut/magic", serde_json::json!(78)),
            ("/zrzut/symbol", serde_json::json!("XAUUSD.s")),
        ] {
            let mut bad = original.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            conduit_server::store::write_json_atomic(&path, &bad).unwrap();
            assert!(wczytaj_scoped(&ws, 123, "broker", 0, 77, "XAUUSD").is_none(), "{pointer}");
        }
        std::fs::write(&path, b"{broken").unwrap();
        assert!(wczytaj_scoped(&ws, 123, "broker", 0, 77, "XAUUSD").is_none());
    }

    #[test]
    fn scoped_rejects_unknown_identity_and_never_uses_server_as_a_path() {
        let ws = scoped_test_workspace("invalid");
        for (login, server, mode, symbol) in [(0, "broker", 0, "XAUUSD"),
            (123, " ", 0, "XAUUSD"), (123, "broker", -1, "XAUUSD"),
            (123, "broker", 3, "XAUUSD"), (123, "broker", 0, "")] {
            assert!(zapisz_scoped(&ws, &[], 1, login, 77, symbol, 90, server, mode).is_err());
            assert!(wczytaj_scoped(&ws, login, server, mode, 77, symbol).is_none());
        }
        let p = sciezka_scoped(&ws, 123, "../../broker/", 0, 77, "../XAUUSD");
        assert_eq!(p.parent().unwrap().parent().unwrap(), ws.root.join("account_state"));
        assert_eq!(p.parent().unwrap().file_name().unwrap().to_str().unwrap().len(), 64);
    }

    /// Sygnał SELL odtworzony z konta ma mieć cele ułożone MALEJĄCO — inaczej
    /// pierwszy etap byłby najdalszym celem i nigdy by się nie zaliczył.
    #[test]
    fn cele_sella_ida_w_dol() {
        let mut o1 = zlec(1, Some(4), 0, 4050.0);
        o1.kind = PendingKind::SellLimit;
        o1.tp = Some(4030.0);
        o1.sl = Some(4062.0);
        let mut o2 = zlec(2, Some(4), 1, 4055.0);
        o2.kind = PendingKind::SellLimit;
        o2.tp = Some(4020.0);
        o2.sl = Some(4060.0);
        let w = odtworz(None, &[], &[o1, o2], 0, 770_077, "XAUUSD", "CD");
        let b = &w.koszyki[0];
        assert_eq!(b.side, Side::Sell);
        assert_eq!(b.tps, vec![4030.0, 4020.0]);
        assert_eq!(
            b.sl,
            Some(4062.0),
            "dla SELL najbezpieczniejszy SL to najwyższy"
        );
    }
}
