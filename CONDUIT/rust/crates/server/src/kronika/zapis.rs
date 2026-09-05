//! Zapis do JEDNEGO ciągłego pliku.
//!
//! Cała trudność tego modułu mieści się w jednym zdaniu: **plik rośnie tylko
//! w dół i nic w nim nigdy nie jest nadpisywane**. Edycja wiadomości nie
//! podmienia wiersza — dopisuje nowy. Skasowanie nie usuwa wiersza — dopisuje
//! nowy. Dzięki temu z pliku da się odtworzyć, co bot WIDZIAŁ w chwili decyzji,
//! a nie tylko to, co widać w kanale dzisiaj.

use crate::kronika::{czas_iso, Fsync, Rodzaj, Rozpoznanie, Ustawienia, Wpis, SCHEMAT};
use anyhow::{Context, Result};
use std::collections::{HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `YYYY-MM-DD_HHMMSS` w strefie zapisu — nazwa kopii zapasowej.
///
/// Sekundy są w nazwie świadomie: dwa przestawienia ścieżki w tej samej
/// minucie nie mają prawa nadpisać sobie kopii, a to jest DOKŁADNIE ten plik,
/// po który sięga się wtedy, gdy coś poszło nie tak.
fn stempel_kopii(ms: i64, strefa_ms: i64) -> String {
    use chrono::{FixedOffset, TimeZone};
    let strefa = FixedOffset::east_opt((strefa_ms / 1000) as i32)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("offset zerowy jest poprawny"));
    strefa
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d_%H%M%S").to_string())
        .unwrap_or_else(|| ms.to_string())
}

const KOPII_MAX: usize = 10;

/// Kasuje najstarsze kopie ponad limit. Nazwy niosą stempel czasu, więc
/// kolejność alfabetyczna jest chronologiczną i nie trzeba pytać systemu
/// o daty plików (które kopiowanie i tak potrafi przestawić).
fn sprzataj_kopie(katalog: &Path) {
    let Ok(rd) = std::fs::read_dir(katalog) else {
        return;
    };
    let mut kopie: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("kronika_backup_") && n.ends_with(".jsonl"))
                .unwrap_or(false)
        })
        .collect();
    if kopie.len() <= KOPII_MAX {
        return;
    }
    kopie.sort();
    for p in &kopie[..kopie.len() - KOPII_MAX] {
        let _ = std::fs::remove_file(p);
    }
}

pub fn sprawdz_zapisywalnosc(sciezka: &Path) -> Result<()> {
    if let Some(dir) = sciezka.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).with_context(|| {
                format!(
                    "katalog {} nie istnieje i nie da się go utworzyć",
                    dir.display()
                )
            })?;
        }
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(sciezka)
        .with_context(|| format!("nie da się pisać do {}", sciezka.display()))?;
    Ok(())
}

const OKNO_PODGLADU: usize = 100;

/// Co rejestrator zrobił z wiadomością.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decyzja {
    Zapisano,
    /// odrzucone przez opcje zapisu — z powodem, bo cisza tu jest zakazana
    Pominieto(&'static str),
    /// rejestrator wyłączony w ustawieniach
    Wylaczona,
}

/// Wiadomość podana do zapisu. Struktura zamiast dwunastu argumentów: przy
/// tylu polach tego samego typu pomyłka w kolejności kompiluje się bez słowa.
#[derive(Debug, Clone)]
pub struct Przychodzace<'a> {
    /// chwila ODEBRANIA — zegar tego procesu, nie Telegrama
    pub odebrano_ms: i64,
    pub rodzaj: Rodzaj,
    pub chat_id: i64,
    pub chat: &'a str,
    pub temat: Option<i64>,
    pub msg_id: i64,
    pub reply_to: Option<i64>,
    pub edit_of: Option<i64>,
    /// znacznik Telegrama w milisekundach
    pub ts_telegram_ms: i64,
    pub text: &'a str,
    pub nasluchiwany: bool,
    pub format: Option<&'a str>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Liczniki {
    pub zapisanych: u64,
    pub nowych: u64,
    pub edycji: u64,
    pub skasowanych: u64,
    /// odrzucone przez opcje zapisu (nie mylić z błędami)
    pub pominietych: u64,
    pub bledow: u64,
    pub ostatni_blad: Option<String>,
    /// rozmiar bieżącego pliku w bajtach
    pub bajtow: u64,
    pub obrotow: u64,
    pub start_ms: i64,
    /// kiedy wpadło ostatnie zdarzenie — 0, gdy jeszcze nic
    pub ostatnie_ms: i64,
}

/// Rejestrator. Jeden na proces; nie jest `Sync` sam z siebie, więc dzieli się
/// go `Mutexem` (tak robi to i `kronika.exe`, i Conduit).
pub struct Kronika {
    ust: Ustawienia,
    korzen: PathBuf,
    sciezka: PathBuf,
    plik: Option<File>,
    seq: u64,
    od_fsync: u32,
    pub liczniki: Liczniki,
    ostatnie: VecDeque<Wpis>,
    /// Co zastaliśmy w pliku przy ostatnim otwarciu — do zameldowania
    /// „kontynuuję" i do decyzji o kopii zapasowej.
    rozpoznanie: Rozpoznanie,
    /// Pliki, których kopię zapasową już w tym uruchomieniu zrobiliśmy.
    ///
    /// **Kopia ma powstać RAZ na uruchomienie, nie przy każdym wpisie** —
    /// inaczej rejestrator, który dostaje kilkaset wiadomości dziennie,
    /// zapełniłby dysk kopiami rosnącego pliku. Zbiór, a nie `bool`, bo
    /// ścieżkę wolno przestawić w locie i NOWY plik też zasługuje na kopię.
    kopie: HashSet<PathBuf>,
    /// Ścieżka ostatniej wykonanej kopii — panel ma powiedzieć, gdzie ona jest.
    ostatnia_kopia: Option<PathBuf>,
}

impl Kronika {
    /// Otwiera (albo tworzy) plik i — jeśli włączone — dopisuje znacznik startu.
    ///
    /// `korzen` to katalog, względem którego liczy się ścieżka z ustawień.
    pub fn otworz(ust: Ustawienia, korzen: impl Into<PathBuf>, teraz_ms: i64) -> Result<Self> {
        let korzen = korzen.into();
        let sciezka = ust.sciezka(&korzen);
        let mut k = Kronika {
            ust,
            korzen,
            sciezka,
            plik: None,
            seq: 0,
            od_fsync: 0,
            liczniki: Liczniki {
                start_ms: teraz_ms,
                ..Default::default()
            },
            ostatnie: VecDeque::with_capacity(OKNO_PODGLADU),
            rozpoznanie: Rozpoznanie::default(),
            kopie: HashSet::new(),
            ostatnia_kopia: None,
        };
        // Wyłączony rejestrator nie tworzy nawet pustego pliku: plik o zerowej
        // długości wygląda w katalogu dokładnie tak samo jak plik po awarii
        // zapisu, a to są dwie zupełnie różne diagnozy.
        if k.ust.wlaczona {
            k.upewnij_sie_ze_otwarty()?;
            k.liczniki.bajtow = k.rozmiar();
            if k.ust.znaczniki_sesji {
                let opis = k.opis_sesji();
                k.znacznik(Rodzaj::Start, teraz_ms, &opis)?;
            }
        }
        Ok(k)
    }

    pub fn ustawienia(&self) -> &Ustawienia {
        &self.ust
    }

    pub fn sciezka(&self) -> &Path {
        &self.sciezka
    }

    /// Co zastaliśmy w pliku przy jego ostatnim otwarciu.
    pub fn rozpoznanie(&self) -> &Rozpoznanie {
        &self.rozpoznanie
    }

    /// Gdzie leży kopia zapasowa zrobiona przy tym otwarciu (jeśli w ogóle).
    pub fn ostatnia_kopia(&self) -> Option<&Path> {
        self.ostatnia_kopia.as_deref()
    }

    /// Ostatnie wiersze — od najnowszego. Podgląd „co wpada".
    pub fn ostatnie(&self, ile: usize) -> Vec<Wpis> {
        self.ostatnie.iter().rev().take(ile).cloned().collect()
    }

    pub fn rozmiar(&self) -> u64 {
        std::fs::metadata(&self.sciezka)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Podmienia opcje zapisu w locie.
    ///
    /// Zmiana ścieżki zamyka stary plik i otwiera nowy — **bez przenoszenia
    /// treści**. Stary plik zostaje tam, gdzie był; przeniesienie danych za
    /// plecami użytkownika byłoby jedyną operacją w tym module, która coś
    /// nadpisuje.
    pub fn przestaw(&mut self, nowe: Ustawienia, teraz_ms: i64) -> Result<()> {
        nowe.sprawdz().map_err(|e| anyhow::anyhow!(e))?;
        let nowa_sciezka = nowe.sciezka(&self.korzen);
        let zmiana_pliku = nowa_sciezka != self.sciezka;
        // ŚCIEŻKĘ SPRAWDZAMY, ZANIM PORZUCIMY STARY PLIK.
        //
        // Wcześniej kolejność była odwrotna: zamknij stary → otwórz nowy →
        // (jeśli się nie da) zwróć błąd. Rejestrator zostawał wtedy BEZ
        // ŻADNEGO otwartego pliku, a komunikat mówił „do restartu pracuje
        // poprzedni plik" — czyli nieprawdę. Katalog bez prawa zapisu (albo
        // literówka w ścieżce z panelu) kosztował cały strumień do restartu,
        // po cichu.
        if zmiana_pliku && nowe.wlaczona {
            sprawdz_zapisywalnosc(&nowa_sciezka)?;
        }
        if zmiana_pliku {
            if self.ust.znaczniki_sesji {
                let _ = self.znacznik(Rodzaj::Stop, teraz_ms, "zmiana pliku zapisu");
            }
            self.zamknij();
            self.sciezka = nowa_sciezka;
        }
        self.ust = nowe;
        if self.ust.wlaczona {
            self.upewnij_sie_ze_otwarty()?;
            self.liczniki.bajtow = self.rozmiar();
            if zmiana_pliku && self.ust.znaczniki_sesji {
                let opis = self.opis_sesji();
                self.znacznik(Rodzaj::Start, teraz_ms, &opis)?;
            }
        } else {
            self.zamknij();
        }
        Ok(())
    }

    /// Dopisuje wiadomość. Zwraca decyzję, żeby wywołujący mógł POLICZYĆ
    /// pominięte — bo „nic się nie zapisało" i „nic nie przyszło" to dwie
    /// zupełnie różne rzeczy, a wyglądają tak samo.
    pub fn zapisz(&mut self, p: Przychodzace<'_>, rozpoznane: bool) -> Result<Decyzja> {
        if !self.ust.wlaczona {
            return Ok(Decyzja::Wylaczona);
        }
        if !self.ust.zrodla.pasuje(p.chat_id, p.temat) {
            self.liczniki.pominietych += 1;
            return Ok(Decyzja::Pominieto(
                "kanał nie jest zaznaczony do nagrywania",
            ));
        }
        if !self.ust.puste && p.text.trim().is_empty() {
            self.liczniki.pominietych += 1;
            return Ok(Decyzja::Pominieto("wiadomość bez treści tekstowej"));
        }
        // Nierozpoznane odrzucamy TYLKO dla wiadomości nowych i edycji.
        // Skasowanie nigdy nie ma treści, więc parser nigdy go nie rozpozna —
        // ta reguła wycięłaby całą klasę zdarzeń, której najbardziej brakuje.
        if !self.ust.nierozpoznane && !rozpoznane && p.rodzaj != Rodzaj::Skasowana {
            self.liczniki.pominietych += 1;
            return Ok(Decyzja::Pominieto("parser nie rozpoznał treści"));
        }

        self.seq += 1;
        let w = Wpis {
            v: SCHEMAT,
            seq: self.seq,
            rodzaj: p.rodzaj,
            odebrano_ms: p.odebrano_ms,
            odebrano: czas_iso(p.odebrano_ms, self.ust.strefa_ms()),
            ts_telegram_ms: p.ts_telegram_ms,
            chat_id: p.chat_id,
            chat: p.chat.to_string(),
            temat: p.temat,
            msg_id: p.msg_id,
            reply_to: p.reply_to,
            edit_of: p.edit_of,
            text: p.text.to_string(),
            znakow: p.text.chars().count(),
            nasluchiwany: p.nasluchiwany,
            format: p.format.map(|s| s.to_string()),
            rozpoznane,
            uwaga: None,
            ts_telegram: None,
        };
        self.dopisz(&w)?;

        self.liczniki.zapisanych += 1;
        self.liczniki.ostatnie_ms = p.odebrano_ms;
        match p.rodzaj {
            Rodzaj::Nowa => self.liczniki.nowych += 1,
            Rodzaj::Edycja => self.liczniki.edycji += 1,
            Rodzaj::Skasowana => self.liczniki.skasowanych += 1,
            _ => {}
        }
        if self.ostatnie.len() == OKNO_PODGLADU {
            self.ostatnie.pop_front();
        }
        self.ostatnie.push_back(w);
        Ok(Decyzja::Zapisano)
    }

    /// Znacznik sesji — `start` przy otwarciu, `stop` przy uprzejmym zamknięciu.
    pub fn znacznik(&mut self, rodzaj: Rodzaj, teraz_ms: i64, opis: &str) -> Result<()> {
        if !self.ust.znaczniki_sesji || self.plik.is_none() {
            return Ok(());
        }
        self.seq += 1;
        let w = Wpis {
            v: SCHEMAT,
            seq: self.seq,
            rodzaj,
            odebrano_ms: teraz_ms,
            odebrano: czas_iso(teraz_ms, self.ust.strefa_ms()),
            ts_telegram_ms: 0,
            chat_id: 0,
            chat: String::new(),
            temat: None,
            msg_id: 0,
            reply_to: None,
            edit_of: None,
            text: String::new(),
            znakow: 0,
            nasluchiwany: false,
            format: None,
            rozpoznane: false,
            uwaga: Some(opis.to_string()),
            ts_telegram: None,
        };
        self.dopisz(&w)
    }

    /// Uprzejme zamknięcie: znacznik `stop` i `fsync`. Wołane przy Ctrl+C.
    pub fn zakoncz(&mut self, teraz_ms: i64, powod: &str) {
        let _ = self.znacznik(Rodzaj::Stop, teraz_ms, powod);
        if let Some(f) = self.plik.as_mut() {
            let _ = f.sync_all();
        }
        self.zamknij();
    }

    // ---------- środek ----------

    fn opis_sesji(&self) -> String {
        let zrodla = match self.ust.zrodla.ile() {
            None => "wszystkie kanały".to_string(),
            Some(n) => format!("{n} zaznaczonych źródeł"),
        };
        format!(
            "kronika v{} · schemat {SCHEMAT} · {zrodla} · nierozpoznane: {} · fsync: {:?}",
            env!("CARGO_PKG_VERSION"),
            if self.ust.nierozpoznane {
                "zapisuję"
            } else {
                "pomijam"
            },
            self.ust.fsync
        )
    }

    fn upewnij_sie_ze_otwarty(&mut self) -> Result<()> {
        if self.plik.is_some() {
            return Ok(());
        }
        if let Some(dir) = self.sciezka.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("nie udało się utworzyć {}", dir.display()))?;
            }
        }
        // ROZPOZNANIE PRZED OTWARCIEM, KOPIA PRZED PIERWSZYM DOPISANIEM.
        //
        // Kolejność jest jedyną możliwą: `OpenOptions::append` sam z siebie
        // pliku nie psuje, ale gdyby zmiana formatu między wersjami bota
        // okazała się niezgodna, jedyną kopią zdatną do ratunku jest ta
        // sprzed pierwszej dopisanej linii.
        self.rozpoznanie = crate::kronika::rozpoznaj(&self.sciezka);
        self.zrob_kopie_raz();

        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.sciezka)
            .with_context(|| format!("nie udało się otworzyć {}", self.sciezka.display()))?;
        self.plik = Some(f);
        Ok(())
    }

    /// Kopia zapasowa istniejącego pliku do `logs/` **katalogu bota**.
    ///
    /// Świadomie do katalogu bota, a nie obok oryginału: plik kroniki ma
    /// odtąd mieszkać na pulpicie użytkownika i zasypywanie pulpitu kopiami
    /// z każdego uruchomienia byłoby karą za włączenie funkcji. Kopia jest
    /// zabezpieczeniem NA CZAS podmiany wersji, a nie archiwum.
    ///
    /// Cicha przy błędzie **z rozmysłem**: nieudana kopia nie może wstrzymać
    /// zapisu strumienia — strumień jest nie do odtworzenia, kopia tak.
    /// Niepowodzenie ląduje w licznikach i w logu technicznym.
    fn zrob_kopie_raz(&mut self) {
        if !self.rozpoznanie.istnial || self.rozpoznanie.bajtow == 0 {
            return;
        }
        if !self.kopie.insert(self.sciezka.clone()) {
            return; // ten plik ma już kopię w tym uruchomieniu
        }
        let katalog = self.korzen.join("logs");
        if let Err(e) = std::fs::create_dir_all(&katalog) {
            tracing::warn!(blad = %e, "kronika: nie udało się utworzyć katalogu na kopię");
            return;
        }
        let cel = katalog.join(format!(
            "kronika_backup_{}.jsonl",
            stempel_kopii(crate::kronika::teraz_ms(), self.ust.strefa_ms())
        ));
        match std::fs::copy(&self.sciezka, &cel) {
            Ok(bajtow) => {
                tracing::info!(
                    zrodlo = %self.sciezka.display(),
                    kopia = %cel.display(),
                    bajtow,
                    "kronika: kopia zapasowa przed pierwszym dopisaniem"
                );
                self.ostatnia_kopia = Some(cel);
            }
            Err(e) => {
                tracing::warn!(blad = %e, cel = %cel.display(), "kronika: kopia zapasowa NIE POWSTAŁA");
                self.liczniki.ostatni_blad = Some(format!("kopia zapasowa nie powstała: {e}"));
            }
        }
        sprzataj_kopie(&katalog);
    }

    fn zamknij(&mut self) {
        if let Some(f) = self.plik.take() {
            let _ = f.sync_all();
        }
    }

    fn dopisz(&mut self, w: &Wpis) -> Result<()> {
        self.upewnij_sie_ze_otwarty()?;
        // JEDNA linia budowana w całości i wypychana JEDNYM zapisem. Dopisywanie
        // w trybie `append` jest niepodzielne, więc wiersze nie wchodzą sobie
        // w środek nawet wtedy, gdy do pliku pisze drugi proces.
        let mut linia = serde_json::to_string(w)?;
        linia.push('\n');
        let dlugosc = linia.len() as u64;
        let wynik = {
            let f = self.plik.as_mut().expect("plik otwarty linijkę wyżej");
            f.write_all(linia.as_bytes())
        };
        if let Err(e) = wynik {
            self.liczniki.bledow += 1;
            self.liczniki.ostatni_blad = Some(e.to_string());
            return Err(anyhow::Error::from(e).context("zapis kroniki"));
        }
        self.liczniki.bajtow += dlugosc;
        self.zsynchronizuj()?;
        self.obroc_jesli_trzeba()?;
        Ok(())
    }

    fn zsynchronizuj(&mut self) -> Result<()> {
        let trzeba = match self.ust.fsync {
            Fsync::Kazda => true,
            Fsync::Nigdy => false,
            Fsync::Co { n } => {
                self.od_fsync += 1;
                if self.od_fsync >= n.max(1) {
                    self.od_fsync = 0;
                    true
                } else {
                    false
                }
            }
        };
        if trzeba {
            if let Some(f) = self.plik.as_mut() {
                f.sync_data().context("fsync kroniki")?;
            }
        }
        Ok(())
    }

    /// Obrót pliku po przekroczeniu rozmiaru.
    ///
    /// Obrócony plik dostaje w nazwie znacznik czasu, więc **kolejność
    /// alfabetyczna jest kolejnością chronologiczną** i czytnik nie musi
    /// niczego zgadywać. Bieżący plik zawsze nazywa się tak, jak w ustawieniach.
    fn obroc_jesli_trzeba(&mut self) -> Result<()> {
        if self.ust.obrot_mb == 0 {
            return Ok(());
        }
        let limit = self.ust.obrot_mb.saturating_mul(1024 * 1024);
        if self.liczniki.bajtow < limit {
            return Ok(());
        }
        self.zamknij();
        let cel = self.nazwa_obrotu(crate::kronika::teraz_ms());
        std::fs::rename(&self.sciezka, &cel)
            .with_context(|| format!("obrót pliku kroniki do {}", cel.display()))?;
        self.liczniki.obrotow += 1;
        self.liczniki.bajtow = 0;
        self.upewnij_sie_ze_otwarty()?;
        self.sprzataj();
        Ok(())
    }

    fn nazwa_obrotu(&self, ms: i64) -> PathBuf {
        use chrono::{FixedOffset, TimeZone};
        let strefa = FixedOffset::east_opt((self.ust.strefa_ms() / 1000) as i32)
            .unwrap_or_else(|| FixedOffset::east_opt(0).expect("offset zerowy jest poprawny"));
        let stempel = strefa
            .timestamp_millis_opt(ms)
            .single()
            .map(|t| t.format("%Y%m%d-%H%M%S").to_string())
            .unwrap_or_else(|| ms.to_string());
        let rdzen = self
            .sciezka
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("kronika");
        let rozsz = self
            .sciezka
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("jsonl");
        let katalog = self.sciezka.parent().unwrap_or_else(|| Path::new("."));
        katalog.join(format!("{rdzen}-{stempel}.{rozsz}"))
    }

    /// Kasuje najstarsze obrócone pliki ponad limit. Bieżącego nie dotyka.
    fn sprzataj(&self) {
        if self.ust.trzymaj_plikow == 0 {
            return;
        }
        let mut stare = crate::kronika::odczyt::pliki_kroniki(&self.sciezka);
        stare.retain(|p| p != &self.sciezka);
        let limit = self.ust.trzymaj_plikow as usize;
        if stare.len() <= limit {
            return;
        }
        for p in &stare[..stare.len() - limit] {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl Drop for Kronika {
    fn drop(&mut self) {
        // Bez znacznika `stop`: `Drop` bywa wołany przy zwijaniu paniki, a wtedy
        // zapis mógłby panikować powtórnie. Znacznik dopisuje `zakoncz`.
        self.zamknij();
    }
}

#[cfg(test)]
mod testy {
    use super::*;
    use crate::kronika::{Zrodla, Zrodlo};

    fn katalog(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "kronika-{tag}-{}-{}",
            std::process::id(),
            crate::kronika::teraz_ms()
        ));
        p
    }

    fn wiadomosc<'a>(ms: i64, rodzaj: Rodzaj, msg_id: i64, text: &'a str) -> Przychodzace<'a> {
        Przychodzace {
            odebrano_ms: ms,
            rodzaj,
            chat_id: -100,
            chat: "ATFX",
            temat: None,
            msg_id,
            reply_to: None,
            edit_of: if rodzaj == Rodzaj::Edycja {
                Some(msg_id)
            } else {
                None
            },
            ts_telegram_ms: ms,
            text,
            nasluchiwany: true,
            format: Some("ATFX"),
        }
    }

    fn bez_znacznikow() -> Ustawienia {
        Ustawienia {
            znaczniki_sesji: false,
            ..Default::default()
        }
    }

    fn linie(p: &Path) -> Vec<Wpis> {
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn wszystko_lezy_w_jednym_pliku_i_w_kolejnosci_odebrania() {
        // TO JEST CAŁE WYMAGANIE: jeden plik, wszystkie kanały i tematy po kolei.
        let dir = katalog("jeden-plik");
        let t = 1_785_000_000_000;
        {
            let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
            let mut a = wiadomosc(t, Rodzaj::Nowa, 1, "z kanału A");
            a.chat_id = -100;
            a.chat = "A";
            k.zapisz(a, true).unwrap();

            let mut b = wiadomosc(t + 1000, Rodzaj::Nowa, 2, "z tematu 7 kanału B");
            b.chat_id = -200;
            b.chat = "B";
            b.temat = Some(7);
            k.zapisz(b, true).unwrap();

            let mut c = wiadomosc(t + 2000, Rodzaj::Nowa, 3, "z tematu 9 kanału B");
            c.chat_id = -200;
            c.chat = "B";
            c.temat = Some(9);
            k.zapisz(c, true).unwrap();
        }
        // doba minęła — a plik JEST TEN SAM
        {
            let mut k = Kronika::otworz(bez_znacznikow(), &dir, t + 86_400_000).unwrap();
            k.zapisz(
                wiadomosc(t + 86_400_000, Rodzaj::Nowa, 4, "nazajutrz"),
                true,
            )
            .unwrap();
        }

        let pliki: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".jsonl"))
            .collect();
        assert_eq!(
            pliki,
            vec!["kronika.jsonl"],
            "ma być JEDEN plik, także po zmianie doby"
        );
        // …a drugie uruchomienie zostawiło KOPIĘ ZAPASOWĄ w `logs/` — jedną,
        // przed pierwszym dopisaniem, nie po każdym wpisie.
        let kopie: Vec<_> = std::fs::read_dir(dir.join("logs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("kronika_backup_"))
            .collect();
        assert_eq!(
            kopie.len(),
            1,
            "kopia ma powstać RAZ na uruchomienie: {kopie:?}"
        );

        let w = linie(&dir.join("kronika.jsonl"));
        assert_eq!(w.len(), 4, "restart procesu DOPISUJE, nie przewija");
        let kolejnosc: Vec<i64> = w.iter().map(|x| x.odebrano_ms).collect();
        let mut posortowane = kolejnosc.clone();
        posortowane.sort_unstable();
        assert_eq!(
            kolejnosc, posortowane,
            "kolejność w pliku = kolejność odebrania"
        );
        // każdy wiersz jednoznacznie niesie, skąd pochodzi
        assert_eq!((w[1].chat_id, w[1].temat), (-200, Some(7)));
        assert_eq!((w[2].chat_id, w[2].temat), (-200, Some(9)));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn edycja_dopisuje_wiersz_a_nie_podmienia() {
        let dir = katalog("edycja");
        let t = 1_785_000_000_000;
        {
            let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
            k.zapisz(wiadomosc(t, Rodzaj::Nowa, 500, "RISK FREE 4057"), true)
                .unwrap();
            k.zapisz(
                wiadomosc(
                    t + 141_000,
                    Rodzaj::Edycja,
                    500,
                    "RISK FREE 4057\nTP1 4060\nTP2 4065",
                ),
                true,
            )
            .unwrap();
        }
        let w = linie(&dir.join("kronika.jsonl"));
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].rodzaj, Rodzaj::Nowa);
        assert_eq!(
            w[0].text, "RISK FREE 4057",
            "pierwsza wersja MUSI zostać nienaruszona"
        );
        assert_eq!(w[1].rodzaj, Rodzaj::Edycja);
        assert_eq!(w[1].edit_of, Some(500));
        assert!(w[1].odebrano_ms > w[0].odebrano_ms);
        assert!(
            w[1].znakow > w[0].znakow,
            "drabinka dopisana edycją wydłuża treść"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tresc_wielolinijkowa_zostaje_jednym_wierszem() {
        let dir = katalog("wielolinia");
        let t = 1_785_000_000_000;
        {
            let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
            k.zapisz(
                wiadomosc(t, Rodzaj::Nowa, 1, "BUY GOLD\nTP1 4030\nTP2 4040\nSL 4010"),
                true,
            )
            .unwrap();
        }
        let tresc = std::fs::read_to_string(dir.join("kronika.jsonl")).unwrap();
        assert_eq!(tresc.lines().count(), 1, "jedno zdarzenie = jeden wiersz");
        let w = linie(&dir.join("kronika.jsonl"));
        assert_eq!(w[0].text.lines().count(), 4, "treść wraca w całości");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wybor_kanalow_odrzuca_reszte_i_to_liczy() {
        let dir = katalog("wybor");
        let t = 1_785_000_000_000;
        let ust = Ustawienia {
            znaczniki_sesji: false,
            zrodla: Zrodla::Wybrane {
                lista: vec![Zrodlo {
                    chat_id: -100,
                    temat: None,
                }],
            },
            ..Default::default()
        };
        let mut k = Kronika::otworz(ust, &dir, t).unwrap();
        assert_eq!(
            k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "wolno"), true)
                .unwrap(),
            Decyzja::Zapisano
        );

        let mut obcy = wiadomosc(t + 1, Rodzaj::Nowa, 2, "nie wolno");
        obcy.chat_id = -999;
        assert!(matches!(
            k.zapisz(obcy, true).unwrap(),
            Decyzja::Pominieto(_)
        ));
        assert_eq!(k.liczniki.zapisanych, 1);
        assert_eq!(
            k.liczniki.pominietych, 1,
            "pominięcie musi być POLICZONE, nie przemilczane"
        );
        drop(k);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nierozpoznane_domyslnie_wchodza_a_skasowanie_zawsze() {
        let dir = katalog("nierozpoznane");
        let t = 1_785_000_000_000;
        let ust = Ustawienia {
            znaczniki_sesji: false,
            nierozpoznane: false,
            ..Default::default()
        };
        let mut k = Kronika::otworz(ust, &dir, t).unwrap();
        assert!(matches!(
            k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "dzień dobry"), false)
                .unwrap(),
            Decyzja::Pominieto(_)
        ));
        // skasowanie NIGDY nie ma treści — reguła „tylko rozpoznane" wycięłaby
        // całą klasę zdarzeń, której najbardziej brakuje
        assert_eq!(
            k.zapisz(wiadomosc(t + 1, Rodzaj::Skasowana, 1, ""), false)
                .unwrap(),
            Decyzja::Zapisano
        );
        drop(k);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn znaczniki_sesji_zamykaja_dziure_w_pliku() {
        // Bez nich cisza na kanałach wygląda tak samo jak wyłączony rejestrator.
        let dir = katalog("znaczniki");
        let t = 1_785_000_000_000;
        {
            let mut k = Kronika::otworz(Ustawienia::default(), &dir, t).unwrap();
            k.zapisz(wiadomosc(t + 10, Rodzaj::Nowa, 1, "cokolwiek"), true)
                .unwrap();
            k.zakoncz(t + 20, "Ctrl+C");
        }
        let w = linie(&dir.join("kronika.jsonl"));
        assert_eq!(w[0].rodzaj, Rodzaj::Start);
        assert!(w[0].uwaga.as_deref().unwrap().contains("schemat"));
        assert_eq!(w[2].rodzaj, Rodzaj::Stop);
        assert_eq!(w[2].uwaga.as_deref(), Some("Ctrl+C"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn obrot_zachowuje_kolejnosc_chronologiczna_w_nazwach() {
        let dir = katalog("obrot");
        let t = 1_785_000_000_000;
        let ust = Ustawienia {
            znaczniki_sesji: false,
            obrot_mb: 1,
            ..Default::default()
        };
        let mut k = Kronika::otworz(ust, &dir, t).unwrap();
        // 1 MB linii po ~1 kB
        let dlugi = "x".repeat(1024);
        for i in 0..1100 {
            k.zapisz(wiadomosc(t + i, Rodzaj::Nowa, i, &dlugi), true)
                .unwrap();
        }
        assert!(
            k.liczniki.obrotow >= 1,
            "plik powyżej 1 MB musi się obrócić"
        );
        drop(k);

        let pliki = crate::kronika::odczyt::pliki_kroniki(&dir.join("kronika.jsonl"));
        assert!(pliki.len() >= 2);
        assert_eq!(
            pliki.last().unwrap(),
            &dir.join("kronika.jsonl"),
            "bieżący plik jest OSTATNI w kolejności czytania"
        );
        let _ = std::fs::remove_dir_all(dir);
    }


    /// ŚWIEŻY START: nowy plik, ŻADNEJ kopii zapasowej i jasny meldunek.
    ///
    /// Kopia z niczego byłaby gorsza niż jej brak — po tygodniu katalog
    /// `logs/` miałby kilkadziesiąt pustych plików, a ten jeden, który
    /// naprawdę coś ratuje, utonąłby między nimi.
    #[test]
    fn swiezy_start_zaklada_plik_i_nie_robi_kopii() {
        let dir = katalog("swiezy");
        let t = 1_785_000_000_000;
        let k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
        assert!(
            !k.rozpoznanie().istnial,
            "pliku nie było, więc nie ma czego kontynuować"
        );
        assert!(k.ostatnia_kopia().is_none(), "nie ma z czego robić kopii");
        assert!(k
            .rozpoznanie()
            .zdanie(k.sciezka())
            .contains("zakładam nowy plik"));
        drop(k);
        assert!(
            !dir.join("logs").exists(),
            "świeży start nie zaśmieca katalogu kopiami"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nowa_wersja_kontynuuje_istniejacy_plik_i_robi_kopie_raz() {
        let dir = katalog("kontynuacja");
        let t = 1_785_000_000_000;
        // „poprzednia wersja bota"
        {
            let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
            k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "sprzed aktualizacji"), true)
                .unwrap();
            k.zapisz(wiadomosc(t + 1000, Rodzaj::Nowa, 2, "też sprzed"), true)
                .unwrap();
        }
        // „nowa paczka" — ten sam plik, inny proces
        let mut k = Kronika::otworz(bez_znacznikow(), &dir, t + 86_400_000).unwrap();
        let r = k.rozpoznanie().clone();
        assert!(r.istnial);
        assert_eq!(r.wierszy, 2, "ma policzyć, co zastał");
        assert_eq!(r.ostatni_ms, t + 1000, "ma znać czas ostatniego wpisu");
        assert!(!r.obcy_format, "ten sam schemat co nasz");
        let zdanie = r.zdanie(k.sciezka());
        assert!(zdanie.contains("kontynuuję istniejący plik"), "{zdanie}");
        assert!(zdanie.contains("2 wpisów"), "{zdanie}");

        // kopia PRZED dopisaniem — dokładnie jedna, także po wielu wpisach
        let kopia = k
            .ostatnia_kopia()
            .expect("kopia MUSI powstać")
            .to_path_buf();
        assert_eq!(std::fs::read_to_string(&kopia).unwrap().lines().count(), 2);
        for i in 0..5 {
            k.zapisz(
                wiadomosc(t + 86_400_000 + i, Rodzaj::Nowa, 10 + i, "po aktualizacji"),
                true,
            )
            .unwrap();
        }
        let kopii = std::fs::read_dir(dir.join("logs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("kronika_backup_")
            })
            .count();
        assert_eq!(
            kopii, 1,
            "kopia RAZ na uruchomienie, nie przy każdym wpisie"
        );

        drop(k);
        assert_eq!(
            linie(&dir.join("kronika.jsonl")).len(),
            7,
            "2 stare + 5 nowych, nic nie zginęło"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// NIEZNANY FORMAT: nie psujemy pliku i nie milczymy.
    ///
    /// Plik zapisany NOWSZĄ wersją schematu może mieć pola, których ten
    /// czytnik nie rozumie. Odmowa zapisu kosztowałaby strumień (nie do
    /// nadrobienia), a ciche dopisanie — zaufanie do statystyki. Więc:
    /// dopisujemy, robimy kopię i mówimy o tym wprost.
    #[test]
    fn nowszy_format_nie_zatrzymuje_zapisu_ale_ostrzega() {
        let dir = katalog("format");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("kronika.jsonl"),
            "{\"v\":99,\"rodzaj\":\"nowa\",\"odebrano_ms\":1785000000000,\"odebrano\":\"2026-08-04\",\"text\":\"z przyszłości\"}\n",
        )
        .unwrap();
        let mut k = Kronika::otworz(bez_znacznikow(), &dir, 1_785_000_100_000).unwrap();
        assert!(k.rozpoznanie().obcy_format, "wersja 99 > {SCHEMAT}");
        let zdanie = k.rozpoznanie().zdanie(k.sciezka());
        assert!(zdanie.contains("FORMAT NOWSZY NIŻ ZNANY"), "{zdanie}");
        assert!(
            k.ostatnia_kopia().is_some(),
            "obcy format tym bardziej wymaga kopii"
        );
        // …i zapis DZIAŁA
        k.zapisz(
            wiadomosc(1_785_000_100_000, Rodzaj::Nowa, 1, "nasza linia"),
            true,
        )
        .unwrap();
        drop(k);
        assert_eq!(linie(&dir.join("kronika.jsonl")).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// ZMIANA ŚCIEŻKI W LOCIE: stary plik zostaje, nowy dostaje kopię.
    #[test]
    fn zmiana_sciezki_robi_kopie_nowego_pliku() {
        let dir = katalog("przeniesienie");
        let t = 1_785_000_000_000;
        std::fs::create_dir_all(dir.join("cel")).unwrap();
        std::fs::write(
            dir.join("cel/pulpit.jsonl"),
            "{\"v\":2,\"rodzaj\":\"nowa\",\"odebrano_ms\":1785000000000,\"text\":\"stare archiwum\"}\n",
        )
        .unwrap();

        let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
        k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "w starym"), true)
            .unwrap();
        k.przestaw(
            Ustawienia {
                znaczniki_sesji: false,
                plik: dir.join("cel/pulpit.jsonl").display().to_string(),
                ..Default::default()
            },
            t + 1,
        )
        .unwrap();
        assert_eq!(k.rozpoznanie().wierszy, 1, "rozpoznał zastane archiwum");
        assert!(
            k.ostatnia_kopia().is_some(),
            "nowy plik też dostaje kopię przed dopisaniem"
        );
        k.zapisz(wiadomosc(t + 2, Rodzaj::Nowa, 2, "w nowym"), true)
            .unwrap();
        drop(k);

        assert_eq!(
            linie(&dir.join("kronika.jsonl")).len(),
            1,
            "stary plik nietknięty"
        );
        assert_eq!(
            linie(&dir.join("cel/pulpit.jsonl")).len(),
            2,
            "nowy DOPISANY, nie nadpisany"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stare_kopie_zapasowe_sa_sprzatane() {
        let dir = katalog("sprzatanie");
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        for i in 0..15 {
            std::fs::write(
                logs.join(format!("kronika_backup_2026-08-{:02}_120000.jsonl", i + 1)),
                b"{}\n",
            )
            .unwrap();
        }
        // plik obcy — sprzątanie nie ma prawa go ruszyć
        std::fs::write(
            logs.join("kronika.jsonl"),
            b"{\"v\":2,\"rodzaj\":\"nowa\",\"odebrano_ms\":1}\n",
        )
        .unwrap();

        let ust = Ustawienia {
            znaczniki_sesji: false,
            plik: "logs/kronika.jsonl".into(),
            ..Default::default()
        };
        let k = Kronika::otworz(ust, &dir, 1_785_000_000_000).unwrap();
        assert!(k.ostatnia_kopia().is_some());
        drop(k);

        let kopie: Vec<_> = std::fs::read_dir(&logs)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("kronika_backup_"))
            .collect();
        assert_eq!(kopie.len(), 10, "ma zostać dziesięć NAJNOWSZYCH: {kopie:?}");
        assert!(
            logs.join("kronika.jsonl").is_file(),
            "sprzątanie ruszyło plik kroniki!"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// KATALOG BEZ PRAWA ZAPISU: jawny błąd, a rejestrator NIE TRACI
    /// dotychczasowego pliku.
    ///
    /// Wcześniej `przestaw` najpierw zamykał stary plik, a dopiero potem
    /// próbował otworzyć nowy — po nieudanej próbie bot zostawał bez
    /// żadnego zapisu, a komunikat twierdził, że „do restartu pracuje
    /// poprzedni plik".
    #[test]
    fn sciezka_bez_prawa_zapisu_nie_zabiera_dotychczasowego_pliku() {
        let dir = katalog("bezprawa");
        let t = 1_785_000_000_000;
        let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
        k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "działa"), true)
            .unwrap();

        // ścieżka niemożliwa do otwarcia: katalogiem jest ISTNIEJĄCY PLIK
        let blokada = dir.join("kronika.jsonl");
        let zla = blokada.join("nie-da-sie.jsonl");
        let e = k
            .przestaw(
                Ustawienia {
                    znaczniki_sesji: false,
                    plik: zla.display().to_string(),
                    ..Default::default()
                },
                t + 1,
            )
            .expect_err("zapis do nieistniejącego katalogu MUSI być błędem");
        assert!(!e.to_string().is_empty(), "błąd musi mieć treść dla panelu");

        // …i dotychczasowy zapis DZIAŁA DALEJ
        k.zapisz(wiadomosc(t + 2, Rodzaj::Nowa, 2, "nadal działa"), true)
            .unwrap();
        drop(k);
        assert_eq!(
            linie(&dir.join("kronika.jsonl")).len(),
            2,
            "nieudana zmiana nie zabiera zapisu"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn zmiana_pliku_nie_przenosi_starej_tresci() {
        let dir = katalog("przestaw");
        let t = 1_785_000_000_000;
        let mut k = Kronika::otworz(bez_znacznikow(), &dir, t).unwrap();
        k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "stary plik"), true)
            .unwrap();
        k.przestaw(
            Ustawienia {
                znaczniki_sesji: false,
                plik: "inny.jsonl".into(),
                ..Default::default()
            },
            t + 1,
        )
        .unwrap();
        k.zapisz(wiadomosc(t + 2, Rodzaj::Nowa, 2, "nowy plik"), true)
            .unwrap();
        drop(k);

        assert_eq!(
            linie(&dir.join("kronika.jsonl")).len(),
            1,
            "stary plik nietknięty"
        );
        assert_eq!(linie(&dir.join("inny.jsonl")).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wylaczona_kronika_nie_pisze_ale_mowi_o_tym() {
        let dir = katalog("wylaczona");
        let t = 1_785_000_000_000;
        let ust = Ustawienia {
            wlaczona: false,
            znaczniki_sesji: false,
            ..Default::default()
        };
        let mut k = Kronika::otworz(ust, &dir, t).unwrap();
        assert_eq!(
            k.zapisz(wiadomosc(t, Rodzaj::Nowa, 1, "nic"), true)
                .unwrap(),
            Decyzja::Wylaczona
        );
        assert_eq!(k.liczniki.zapisanych, 0);
        drop(k);
        let _ = std::fs::remove_dir_all(dir);
    }
}
