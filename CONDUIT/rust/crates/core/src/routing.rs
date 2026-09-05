
use crate::broker::Broker;
use crate::engine::Engine;
use crate::formaty::{Lancuch, PulapyGlobalne};
use crate::types::{Basket, PendingOrder, Position, Side, SourceKey};
use crate::wielosilnik::{self, ObceObciazenie};
use crate::Settings;
use std::collections::BTreeMap;

// ============================================================
//  WIDOK BROKERA — silnik widzi wyłącznie swoje
// ============================================================

/// KTO JEST WŁAŚCICIELEM KOSZYKA — jedna reguła dla całego programu.
///
/// Musi być JEDNA, bo używają jej dwie rzeczy, które nie mogą się rozjechać:
/// widok brokera (co silnik widzi) i przydział koszyków przy wznowieniu
/// (czym silnik zarządza). Gdyby się różniły, silnik miałby koszyk, którego
/// pozycji nie widzi — uznałby go za pusty, zamknął w statystykach i przestał
/// pilnować, a pozycje zostałyby na rachunku bez opieki.
///
/// * koszyk ze **slotem tego silnika** → jego,
/// * koszyk ze **slotem, którego nikt nie obsługuje** (np. numer sprzed
///   wprowadzenia formatów albo format usunięty z łańcucha) → silnika
///   **zapasowego**,
/// * **brak koszyka** (ręczny bilet z panelu, `basket: None`) → silnika
///   zapasowego.
#[derive(Debug, Clone)]
pub struct Wlasnosc {
    pub slot: u32,
    pub zapasowy: bool,
    /// sloty WSZYSTKICH silników — po to, żeby zapasowy wiedział, co jest
    /// niczyje, a co po prostu cudze
    pub znane_sloty: Vec<u32>,
}

impl Wlasnosc {
    pub fn moj(&self, basket: Option<u32>) -> bool {
        match basket {
            Some(id) => {
                let s = wielosilnik::slot_koszyka(id);
                s == self.slot || (self.zapasowy && !self.znane_sloty.contains(&s))
            }
            None => self.zapasowy,
        }
    }
}

/// Opakowanie brokera pokazujące silnikowi TYLKO jego pozycje i zlecenia.
///
/// Konstrukcja wyjmuje cudze wpisy do schowka, `Drop` je oddaje. Zwrot jest
/// **scalający, nie doklejający**: wpis, który w międzyczasie pojawił się
/// w wektorze brokera (bo terminal go zwrócił), wygrywa ze schowanym. Bez
/// tego pojedynczy zbieg okoliczności zdublowałby pozycję w pamięci bota.
pub struct Widok<'a, B: Broker> {
    inner: &'a mut B,
    kto: Wlasnosc,
    poz: Vec<Position>,
    zle: Vec<PendingOrder>,
    /// ILE RACHUNKU ZABRAŁO SCHOWANIE CUDZYCH POZYCJI — patrz [`Widok::account`].
    /// `(equity, margines)`. Zera znaczą „broker liczy rachunek niezależnie od
    /// wektora pozycji" (tak robi terminal MT5) **albo** „nie ma czego chować"
    /// (jeden silnik) — i w obu wypadkach `account()` jest tożsamościowe.
    ubytek: (f64, f64),
    /// POCZEKALNIA ZAMKNIĘTYCH TRANSAKCJI — patrz [`Widok::drain_closed`].
    poczekalnia: &'a mut Vec<crate::types::ClosedTrade>,
}

impl<'a, B: Broker> Widok<'a, B> {
    pub fn nowy(
        inner: &'a mut B,
        kto: Wlasnosc,
        poczekalnia: &'a mut Vec<crate::types::ClosedTrade>,
    ) -> Self {
        let moje = |basket: Option<u32>| kto.moj(basket);

        // RACHUNEK SPRZED SCHOWANIA — uzasadnienie przy [`Widok::account`].
        let przed = inner.account();

        let mut poz = Vec::new();
        let v = inner.positions_mut();
        let mut i = 0;
        while i < v.len() {
            if moje(v[i].basket) {
                i += 1;
            } else {
                poz.push(v.remove(i));
            }
        }

        let mut zle = Vec::new();
        let v = inner.pendings_mut();
        let mut i = 0;
        while i < v.len() {
            if moje(v[i].basket) {
                i += 1;
            } else {
                zle.push(v.remove(i));
            }
        }

        let po = inner.account();
        let ubytek = (przed.equity - po.equity, przed.margin - po.margin);

        Widok {
            inner,
            kto,
            poz,
            zle,
            ubytek,
            poczekalnia,
        }
    }

    /// Broker pod spodem — do odczytu rzeczy dotyczących CAŁEGO rachunku
    /// (saldo, kwotowanie). Nie do obchodzenia filtra.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn wnetrze(&mut self) -> &mut B {
        self.inner
    }
}

impl<B: Broker> Drop for Widok<'_, B> {
    fn drop(&mut self) {
        let poz = std::mem::take(&mut self.poz);
        let v = self.inner.positions_mut();
        for p in poz {
            if !v.iter().any(|x| x.ticket == p.ticket) {
                v.push(p);
            }
        }
        // Kolejność w wektorze nie ma znaczenia dla logiki, ale ma dla
        // czytelności panelu i dla powtarzalności zrzutów — stąd sortowanie.
        v.sort_by_key(|p| p.ticket);

        let zle = std::mem::take(&mut self.zle);
        let v = self.inner.pendings_mut();
        for o in zle {
            if !v.iter().any(|x| x.ticket == o.ticket) {
                v.push(o);
            }
        }
        v.sort_by_key(|o| o.ticket);
    }
}

impl<B: Broker> Broker for Widok<'_, B> {
    fn quote(&self) -> crate::types::Quote {
        self.inner.quote()
    }
    fn account(&self) -> crate::types::Account {
        let mut a = self.inner.account();
        if self.ubytek.0 == 0.0 && self.ubytek.1 == 0.0 {
            return a;
        }
        a.equity += self.ubytek.0;
        a.margin += self.ubytek.1;
        a.free_margin = a.equity - a.margin;
        a
    }
    fn stops_level(&self) -> f64 {
        self.inner.stops_level()
    }
    fn volume_min(&self) -> f64 {
        self.inner.volume_min()
    }
    fn volume_step(&self) -> f64 {
        self.inner.volume_step()
    }
    fn volume_max(&self) -> f64 {
        self.inner.volume_max()
    }
    fn normalize_order_price(&self, price:f64)->f64 { self.inner.normalize_order_price(price) }
    fn close_receipt_reconciliation_active(&self) -> bool {
        self.inner.close_receipt_reconciliation_active()
    }
    fn close_receipts_pending(&self) -> bool {
        self.inner.close_receipts_pending()
            || (self.close_receipt_reconciliation_active() && !self.poczekalnia.is_empty())
    }
    fn receipt_barrier(&self) -> crate::broker::ReceiptBarrier {
        use crate::broker::ReceiptBarrier;
        match self.inner.receipt_barrier() {
            ReceiptBarrier::RequiresReview => ReceiptBarrier::RequiresReview,
            _ if self.close_receipts_pending() => ReceiptBarrier::Temporary,
            other => other,
        }
    }
    fn execution_session(&self) -> Option<crate::broker::ExecutionSession> { self.inner.execution_session() }
    fn position_identifier(&self, ticket: crate::types::Ticket) -> Option<u64> { self.inner.position_identifier(ticket) }
    fn pending_cancel_snapshot_authoritative(&self) -> bool { self.inner.pending_cancel_snapshot_authoritative() }
    fn cost_net_supported(&self) -> bool { self.inner.cost_net_supported() }
    fn report_cost_consumer_fault(&mut self, reason:&str) {self.inner.report_cost_consumer_fault(reason);}
    fn positions(&self) -> &[Position] {
        self.inner.positions()
    }
    fn pendings(&self) -> &[PendingOrder] {
        self.inner.pendings()
    }
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        self.inner.positions_mut()
    }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        self.inner.pendings_mut()
    }
    /// SCHOWEK — dokładnie to, co konstruktor wyjął z wektorów brokera.
    ///
    /// Jedyne okno na cudze nogi, jakie silnik dostaje, i celowo TYLKO DO
    /// ODCZYTU: liczniki opisujące CAŁY rachunek (poziom marginesu, straż
    /// ekspozycji, sufit portfelowy) mają widzieć komplet, bo margines jest
    /// jeden — a zarządzanie (stopy, domykanie, kasowanie szczebli) dalej
    /// sięga wyłącznie po `positions()`/`pendings()`, czyli po własne.
    /// Uzasadnienie i historia błędu: [`crate::broker::Broker::ukryte_pozycje`].
    ///
    /// Przy jednym silniku oba wektory są puste — kontrakt zera.
    fn ukryte_pozycje(&self) -> &[Position] {
        &self.poz
    }
    fn ukryte_zlecenia(&self) -> &[PendingOrder] {
        &self.zle
    }
    fn open_market(
        &mut self,
        r: crate::broker::OrderReq,
    ) -> crate::broker::BResult<crate::types::Ticket> {
        if self.close_receipts_pending() {
            return Err(crate::broker::BrokerError::Rejected);
        }
        self.inner.open_market(r)
    }
    fn place_pending(
        &mut self,
        r: crate::broker::PendingReq,
    ) -> crate::broker::BResult<crate::types::Ticket> {
        if self.close_receipts_pending() {
            return Err(crate::broker::BrokerError::Rejected);
        }
        self.inner.place_pending(r)
    }
    fn modify_position(
        &mut self,
        t: crate::types::Ticket,
        sl: Option<crate::types::Px>,
        tp: Option<crate::types::Px>,
    ) -> crate::broker::BResult<()> {
        self.inner.modify_position(t, sl, tp)
    }
    fn modify_pending(
        &mut self,
        t: crate::types::Ticket,
        price: crate::types::Px,
        sl: Option<crate::types::Px>,
        tp: Option<crate::types::Px>,
    ) -> crate::broker::BResult<()> {
        self.inner.modify_pending(t, price, sl, tp)
    }
    fn close_position(
        &mut self,
        t: crate::types::Ticket,
        reason: crate::types::CloseReason,
    ) -> crate::broker::BResult<f64> {
        self.inner.close_position(t, reason)
    }
    fn close_partial(
        &mut self,
        t: crate::types::Ticket,
        volume: f64,
        reason: crate::types::CloseReason,
    ) -> crate::broker::BResult<f64> {
        self.inner.close_partial(t, volume, reason)
    }
    fn cancel_pending(&mut self, t: crate::types::Ticket) -> crate::broker::BResult<()> {
        self.inner.cancel_pending(t)
    }
    /// TRANSAKCJE ZAMKNIĘTE SĄ WSPÓLNE I TO JEST PUŁAPKA.
    ///
    /// `drain_closed` u brokera **zabiera** całą listę. Pierwszy silnik, który
    /// ją odczyta, zabrałby także cudze transakcje i zaliczył je do własnych
    /// statystyk — seria strat, wynik dnia, cel dnia i strażnik obsunięcia
    /// liczyłyby się z cudzych zamknięć. Nie byłoby po tym żadnego śladu.
    ///
    /// Dlatego wszystko, co przyszło od brokera, ląduje najpierw w POCZEKALNI
    /// wspólnej dla wszystkich silników, a każdy zabiera z niej wyłącznie swoje.
    /// Reszta czeka na właściciela. Transakcja bez właściciela (slot, którego
    /// nikt nie obsługuje) trafia do silnika zapasowego — nikt jej nie zgubi.
    fn drain_closed(&mut self) -> Vec<crate::types::ClosedTrade> {
        let swieze = self.inner.drain_closed();
        self.poczekalnia.extend(swieze);
        let mut moje = Vec::new();
        let mut i = 0;
        while i < self.poczekalnia.len() {
            if self.kto.moj(self.poczekalnia[i].basket) {
                moje.push(self.poczekalnia.remove(i));
            } else {
                i += 1;
            }
        }
        moje
    }
}

// ============================================================
//  JEDEN SILNIK = JEDEN FORMAT
// ============================================================

/// Silnik przypisany do formatu wraz z jego tożsamością.
pub struct Silnik {
    /// nazwa formatu (`ATFX`, `Synergy`) — klucz łączący kanał, preset i silnik
    pub format: String,
    /// nazwa presetu z aktywnego łańcucha
    pub preset: String,
    /// slot numeracji koszyków
    pub slot: u32,
    /// czy ten silnik przygarnia rzeczy bez koszyka (ręczne bilety z panelu)
    pub zapasowy: bool,
    /// SILNIK „TYLKO-ZARZĄDZANIE" — zamrożony po zmianie łańcucha.
    ///
    /// Powstaje, gdy nowy łańcuch NIE MA nogi dla formatu, którego koszyki
    /// wciąż żyją na rachunku: koszyki są dalej prowadzone STARĄ konfiguracją
    /// (etapy celów, SL, trailing tykają), ale `trasa()` nie kieruje tu
    /// żadnych NOWYCH sygnałów. Bez tego zmiana łańcucha z otwartymi
    /// pozycjami oznaczałaby jedno z dwóch zła: koszyki przejęte przez CUDZY
    /// preset zapasowego silnika albo pozycje bez opieki. Silnik znika sam,
    /// gdy jego ostatni koszyk się domknie i nastąpi kolejna przebudowa.
    pub tylko_zarzadzanie: bool,
    pub z_pliku: bool,
    pub powod: String,
    pub engine: Engine,
}

/// Powód, dla którego wiadomość NIE trafiła do żadnego silnika.
///
/// Cisza jest tu niedopuszczalna: użytkownik musi widzieć, dlaczego sygnał
/// nie został wzięty. Każdy wariant ma własny `kind` w dzienniku decyzji.
#[derive(Debug, Clone, PartialEq)]
pub enum BrakTrasy {
    /// kanał (albo temat forum) nie ma przypisanego formatu
    KanalBezFormatu { kanal: i64, temat: Option<i64> },
    /// format jest, ale aktywny łańcuch nie przypisuje mu presetu
    FormatNieHandluje { format: String, lancuch: String },
    /// format jest w łańcuchu, ale presetu o tej nazwie nie ma na dysku
    PresetNieIstnieje { format: String, preset: String },
}

impl BrakTrasy {
    /// Krótki kod do dziennika decyzji i do licznika odrzutów.
    pub fn kod(&self) -> &'static str {
        match self {
            BrakTrasy::KanalBezFormatu { .. } => "KanalBezFormatu",
            BrakTrasy::FormatNieHandluje { .. } => "FormatNieHandluje",
            BrakTrasy::PresetNieIstnieje { .. } => "PresetNieIstnieje",
        }
    }

    /// Zdanie dla użytkownika — ma powiedzieć, CO KLIKNĄĆ, żeby to naprawić.
    pub fn opis(&self) -> String {
        match self {
            BrakTrasy::KanalBezFormatu { kanal, temat } => format!(
                "Kanał {kanal}{} nie ma przypisanego FORMATU, więc sygnał nie trafił do \
                 żadnego silnika. Wejdź w Kanały i wybierz format dla tego źródła; \
                 pusty wybór znaczy „nasłuchuj, ale nie handluj”.",
                match temat {
                    Some(t) => format!(" (temat {t})"),
                    None => String::new(),
                }
            ),
            BrakTrasy::FormatNieHandluje { format, lancuch } => format!(
                "Format „{format}” nie ma przypisanego presetu w aktywnym łańcuchu \
                 „{lancuch}”, więc ten kanał jest wyłącznie nasłuchiwany. To jest \
                 stan POPRAWNY, jeśli tak został ustawiony — wybierz preset dla tego \
                 formatu, żeby zacząć nim handlować."
            ),
            BrakTrasy::PresetNieIstnieje { format, preset } => format!(
                "Aktywny łańcuch przypisuje formatowi „{format}” preset „{preset}”, \
                 którego NIE MA w katalogu presetów. Sygnał przepadł. Sprawdź nazwę \
                 presetu w łańcuchu albo wgraj brakujący plik."
            ),
        }
    }
}

const FORMAT_ZASTANY: &str = "ATFX";

/// Komplet silników pracujących na jednym rachunku.
pub struct Silniki {
    pub lista: Vec<Silnik>,
    /// nazwa aktywnego łańcucha — do komunikatów
    pub lancuch: String,
    /// Pułapy aktywnego łańcucha. Egzekwuje je RDZEŃ (każdy silnik dostaje
    /// swoją kopię w `Engine::pulapy`); tu leżą do wglądu — dziennik i panel
    /// muszą umieć powiedzieć, jaki sufit obowiązywał.
    #[cfg_attr(not(test), allow(dead_code))]
    pub pulapy: PulapyGlobalne,
    /// formaty, którym trzeba było przesunąć slot z powodu kolizji skrótów
    pub sloty_przesuniete: Vec<String>,
    /// Zamknięte transakcje czekające na swojego właściciela.
    /// Patrz [`Widok::drain_closed`].
    poczekalnia: Vec<crate::types::ClosedTrade>,
}

impl Silniki {
    /// Buduje komplet silników z aktywnego łańcucha.
    ///
    /// `presety` to mapa `nazwa presetu → ustawienia` (z katalogu presetów),
    /// `rachunek` to ustawienia opisujące KONTO (dokument panelu). Dla każdego
    /// formatu składamy jedno z drugim przez
    /// [`crate::wielosilnik::ustawienia_formatu`].
    ///
    /// Zwraca też listę formatów, których presetu nie udało się znaleźć —
    /// wołający ma o nich napisać w dzienniku, a nie przemilczeć.
    pub fn zbuduj(
        lancuch: &Lancuch,
        presety: &BTreeMap<String, Settings>,
        rachunek: &Settings,
        saldo: f64,
    ) -> (Silniki, Vec<BrakTrasy>) {
        let mut braki = Vec::new();
        let formaty: Vec<String> = lancuch
            .presety
            .iter()
            .filter(|(_, p)| !p.is_empty())
            .map(|(f, _)| f.clone())
            .collect();
        let (pary, przesuniete) = wielosilnik::sloty_formatow(&formaty);

        let mut lista: Vec<Silnik> = Vec::new();
        for (format, slot) in pary {
            let nazwa_presetu = lancuch.preset_dla(&format).unwrap_or_default().to_string();
            let Some(p) = presety.get(&nazwa_presetu) else {
                braki.push(BrakTrasy::PresetNieIstnieje {
                    format: format.clone(),
                    preset: nazwa_presetu,
                });
                continue;
            };
            let cfg = wielosilnik::ustawienia_formatu(p, rachunek);
            let mut engine = Engine::new(cfg, saldo);
            engine.przypisz_slot(slot);
            engine.pulapy = lancuch.pulapy.clone();
            lista.push(Silnik {
                powod: String::new(),
                format,
                preset: nazwa_presetu,
                slot,
                zapasowy: false,
                tylko_zarzadzanie: false,
                // ścieżka wielosilnikowa buduje WYŁĄCZNIE z plików presetów —
                // format bez pliku w ogóle nie dostaje silnika (`braki` wyżej)
                z_pliku: true,
                engine,
            });
        }

        // DEGRADACJA ZAMIAST PANIKI (audyt routing.rs:481+817): łańcuch,
        // którego ŻADEN preset nie istnieje na dysku (albo łańcuch bez nóg),
        // dawał pustą listę, a `glowny()` indeksuje `lista[0]` — pętla żywa
        // padała zamiast dalej pilnować rachunku. Silnik awaryjny gra
        // dokumentem panelu, przygarnia wszystko jako zapasowy i NIE bierze
        // nowych sygnałów (`tylko_zarzadzanie`), więc niczego nie otworzy
        // cudzym kompletem ustawień — tylko prowadzi to, co już leży.
        if lista.is_empty() {
            let mut engine = Engine::new(rachunek.clone(), saldo);
            engine.przypisz_slot(wielosilnik::SLOT_STARY);
            engine.pulapy = lancuch.pulapy.clone();
            // Dwa różne powody degradacji to dwie różne naprawy po stronie
            // operatora: brak nóg naprawia się w łańcuchu, brak plików — na dysku.
            let przyczyna = if braki.is_empty() {
                "łańcuch nie ma ani jednej nogi".to_string()
            } else {
                format!("{} nóg bez pliku presetu", braki.len())
            };
            lista.push(Silnik {
                powod: format!(
                    "AWARYJNY: łańcuch „{}” nie dał ani jednego silnika \
                     ({przyczyna}) — tylko zarządzanie zastanym",
                    lancuch.nazwa,
                ),
                format: String::new(),
                preset: String::new(),
                slot: wielosilnik::SLOT_STARY,
                zapasowy: true,
                tylko_zarzadzanie: true,
                z_pliku: false,
                engine,
            });
        }

        // SILNIK ZAPASOWY — dokładnie jeden i zawsze ten sam.
        //
        // Przygarnia wszystko, czego nie da się przypisać po slocie: ręczne
        // bilety otwarte przyciskiem w panelu (`basket: None`), koszyki
        // sprzed wprowadzenia formatów (slot 0) i koszyki formatu, który
        // wypadł z łańcucha. Gdyby zapasowych było dwóch, ta sama ręczna
        // pozycja byłaby zarządzana dwa razy; gdyby nie było żadnego, nie
        // byłaby zarządzana wcale — a to jest dokładnie ta „sierota po
        // restarcie", która kosztowała poprzedni projekt.
        //
        // Wybór jest deterministyczny i nie zmienia się między restartami:
        // najpierw `ATFX` (bo to jego koszyki leżą dziś na rachunku bez
        // prefiksu), a gdy go w łańcuchu nie ma — pierwszy po nazwie.
        let zapas = lista
            .iter()
            .position(|s| s.format == FORMAT_ZASTANY)
            .unwrap_or(0);
        if let Some(s) = lista.get_mut(zapas) {
            s.zapasowy = true;
        }

        (
            Silniki {
                lista,
                lancuch: lancuch.nazwa.clone(),
                pulapy: lancuch.pulapy.clone(),
                sloty_przesuniete: przesuniete,
                poczekalnia: Vec::new(),
            },
            braki,
        )
    }

    pub fn pojedynczy(
        engine: Engine,
        format: String,
        preset: String,
        lancuch: Lancuch,
        z_pliku: bool,
    ) -> Silniki {
        debug_assert_eq!(engine.slot(), wielosilnik::SLOT_STARY);
        Silniki {
            lista: vec![Silnik {
                powod: String::new(),
                format,
                preset,
                slot: engine.slot(),
                zapasowy: true,
                tylko_zarzadzanie: false,
                z_pliku,
                engine,
            }],
            lancuch: lancuch.nazwa.clone(),
            pulapy: lancuch.pulapy,
            sloty_przesuniete: Vec::new(),
            poczekalnia: Vec::new(),
        }
    }

    /// Reguła własności dla silnika o tym indeksie.
    pub fn wlasnosc(&self, i: usize) -> Wlasnosc {
        Wlasnosc {
            slot: self.lista[i].slot,
            zapasowy: self.lista[i].zapasowy,
            znane_sloty: self.lista.iter().map(|s| s.slot).collect(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.lista.is_empty()
    }

    /// Indeks silnika obsługującego dany format (dopasowanie DOSŁOWNE).
    pub fn indeks_formatu(&self, format: &str) -> Option<usize> {
        self.lista.iter().position(|s| s.format == format)
    }

    /// Jak [`Silniki::indeks_formatu`], ale odporne na spacje brzegowe
    /// i wielkość liter — dla nazw przychodzących z KONFIGURACJI kanałów.
    ///
    /// Nazwa formatu w `channels.json` jest wpisywana ręcznie, a budowa
    /// silników bierze nazwę z łańcucha — „atfx ” kontra „ATFX” dawało
    /// silnik, do którego żaden sygnał nigdy nie trafiał (każdy odpadał
    /// jako FormatNieHandluje, audyt routing.rs:553). Dopasowanie dosłowne
    /// idzie PIERWSZE, żeby patologiczna para „ATFX”/„atfx” w jednym
    /// łańcuchu nie zmieniła adresata istniejących tras.
    fn indeks_formatu_luzny(&self, format: &str) -> Option<usize> {
        self.indeks_formatu(format).or_else(|| {
            let f = format.trim();
            self.lista
                .iter()
                .position(|s| s.format.trim().eq_ignore_ascii_case(f))
        })
    }

    /// Indeks silnika, do którego należy koszyk o tym numerze.
    ///
    /// Ta sama reguła co w widoku brokera — dosłownie ta sama funkcja, żeby
    /// nie dało się ich rozjechać.
    pub fn indeks_koszyka(&self, id: u32) -> Option<usize> {
        (0..self.lista.len()).find(|i| self.wlasnosc(*i).moj(Some(id)))
    }

    /// Dokąd trafia wiadomość z tego źródła.
    ///
    /// `format_kanalu` ma zwrócić format przypisany kanałowi albo tematowi
    /// forum (`None` = kanał nie handluje). Rozdzielenie odpowiedzialności
    /// jest celowe: powiązania kanałów mieszkają w serwerze, a ten moduł nie
    /// ma prawa do nich sięgać.
    pub fn trasa(
        &self,
        src: &SourceKey,
        format_kanalu: Option<String>,
    ) -> Result<usize, BrakTrasy> {
        // `trim()` przed testem pustości: format „   ” to brak formatu,
        // nie format o nazwie ze spacji.
        let Some(format) = format_kanalu
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
        else {
            return Err(BrakTrasy::KanalBezFormatu {
                kanal: src.chat_id,
                temat: src.topic_id,
            });
        };
        let i = self
            .indeks_formatu_luzny(&format)
            .ok_or_else(|| BrakTrasy::FormatNieHandluje {
                format: format.clone(),
                lancuch: self.lancuch.clone(),
            })?;
        // Silnik ZAMROŻONY zarządza tym, co ma, ale nowych sygnałów nie
        // bierze — nowy łańcuch świadomie nie dał temu formatowi nogi.
        if self.lista[i].tylko_zarzadzanie {
            return Err(BrakTrasy::FormatNieHandluje {
                format,
                lancuch: self.lancuch.clone(),
            });
        }
        Ok(i)
    }

    /// Liczy obce obciążenie dla KAŻDEGO silnika i wpisuje mu je do pola.
    ///
    /// Woła się raz na obrót pętli, przed przekazaniem kwotowań. Bez tego
    /// pułapy łańcucha widziałyby wyłącznie własny silnik i nie chroniłyby
    /// przed niczym.
    ///
    /// `strona_sygnalu` to kierunek rozważanego właśnie sygnału (`None`, gdy
    /// nie rozważamy żadnego) — służy wyłącznie polu
    /// `przeciwny_kierunek`.
    pub fn przelicz_obce<B: Broker>(&mut self, b: &B, strona_sygnalu: Option<Side>) {
        // Zbieramy fakty ZE WSZYSTKICH silników, potem każdemu podajemy sumę
        // pomniejszoną o jego własny wkład. Dwa przebiegi zamiast n², i —
        // co ważniejsze — nikt nie liczy siebie jako obcego.
        let mut koszyki_razem = 0u32;
        let mut dzis_razem = 0.0f64;
        let wlasne: Vec<(u32, f64)> = self
            .lista
            .iter()
            .map(|s| {
                let k = s.engine.baskets.iter().filter(|x| x.alive()).count() as u32;
                // ZREALIZOWANY wynik NOGI, nie delta equity KONTA: equity jest
                // jedno na rachunek, więc każda z N nóg wnosiła tu pełną dobową
                // deltę całego konta i pułapy dobowe łańcucha odpalały już przy
                // 1/N progu (audyt routing.rs:608). Floating cudzych nóg
                // w limicie dnia to osobna decyzja — engine.rs:10315, fala 2.
                let d = s.engine.stats.realized_today;
                koszyki_razem += k;
                dzis_razem += d;
                (k, d)
            })
            .collect();

        // Loty i LICZBĘ pozycji czytamy z BROKERA, nie z silników: to jest
        // jedyne miejsce, w którym widać, co faktycznie leży na rachunku.
        // Broker jest tu jeszcze nieodfiltrowany (widoki żyją tylko na czas
        // pojedynczego wywołania silnika), więc widzimy komplet.
        let mut buy_slotu: BTreeMap<u32, f64> = BTreeMap::new();
        let mut sell_slotu: BTreeMap<u32, f64> = BTreeMap::new();
        let mut poz_slotu: BTreeMap<u32, u32> = BTreeMap::new();
        let mut buy_razem = 0.0f64;
        let mut sell_razem = 0.0f64;
        let mut poz_razem = 0u32;
        for p in b.positions() {
            let slot = p
                .basket
                .map(wielosilnik::slot_koszyka)
                .unwrap_or(wielosilnik::SLOT_STARY);
            *poz_slotu.entry(slot).or_default() += 1;
            poz_razem += 1;
            match p.side {
                Side::Buy => {
                    *buy_slotu.entry(slot).or_default() += p.volume;
                    buy_razem += p.volume;
                }
                Side::Sell => {
                    *sell_slotu.entry(slot).or_default() += p.volume;
                    sell_razem += p.volume;
                }
            }
        }

        let znane: Vec<u32> = self.lista.iter().map(|s| s.slot).collect();
        for (i, s) in self.lista.iter_mut().enumerate() {
            let (k, d) = wlasne[i];
            let mb = buy_slotu.get(&s.slot).copied().unwrap_or(0.0);
            let ms = sell_slotu.get(&s.slot).copied().unwrap_or(0.0);
            // Pozycje slotu, którego NIKT nie obsługuje, liczą się silnikowi
            // zapasowemu jako własne — bo to on nimi zarządza (`Wlasnosc::moj`).
            let obslugiwane: u32 = poz_slotu
                .iter()
                .filter(|(sl, _)| znane.contains(sl))
                .map(|(_, n)| *n)
                .sum();
            let moje_poz = poz_slotu.get(&s.slot).copied().unwrap_or(0)
                + if s.zapasowy {
                    poz_razem - obslugiwane
                } else {
                    0
                };
            s.engine.obce = ObceObciazenie {
                pozycje: poz_razem.saturating_sub(moje_poz),
                koszyki: koszyki_razem.saturating_sub(k),
                zrealizowane_dzis: dzis_razem - d,
                loty_buy: (buy_razem - mb).max(0.0),
                loty_sell: (sell_razem - ms).max(0.0),
                przeciwny_kierunek: match strona_sygnalu {
                    // „Przeciwny" znaczy: CUDZE pozycje po drugiej stronie.
                    // Własne nie liczą się do konfliktu między formatami —
                    // od tego jest `side_filter` i reguły koszyka.
                    Some(Side::Buy) => (sell_razem - ms) > 0.0,
                    Some(Side::Sell) => (buy_razem - mb) > 0.0,
                    None => false,
                },
            };
        }
    }

    /// Rozdziela koszyki odtworzone po restarcie do właściwych silników.
    ///
    /// Rozstrzyga **wyłącznie slot z numeru koszyka** — czyli dokładnie ta sama
    /// reguła, którą stosuje widok brokera ([`Wlasnosc::moj`]). To nie jest
    /// uproszczenie, tylko warunek poprawności: gdyby przydział koszyków szedł
    /// inną drogą niż widok pozycji, silnik dostałby koszyk, którego pozycji
    /// nie widzi. Uznałby go za pusty, domknął po minucie (`engine.rs`:
    /// sprzątanie koszyków) i przestał pilnować — a pozycje zostałyby na
    /// rachunku z żywym ryzykiem i bez opieki.
    ///
    /// Koszyki sprzed wprowadzenia formatów mają numery bez prefiksu (slot 0),
    /// więc trafiają do silnika **zapasowego**, którym jest `ATFX` — czyli ten
    /// format, z którym te koszyki naprawdę powstały.
    pub fn rozdaj_koszyki(&mut self, koszyki: Vec<Basket>) -> RaportPrzydzialu {
        let mut r = RaportPrzydzialu::default();
        let mut kubelki: Vec<Vec<Basket>> = vec![Vec::new(); self.lista.len()];

        for bk in koszyki {
            let slot = wielosilnik::slot_koszyka(bk.id);
            match self.indeks_koszyka(bk.id) {
                Some(i) => {
                    if self.lista[i].slot == slot {
                        r.po_slocie += 1;
                    } else {
                        r.na_zapas.push(bk.id);
                    }
                    kubelki[i].push(bk);
                }
                None => r.porzucone.push(bk.id),
            }
        }

        for (i, kk) in kubelki.into_iter().enumerate() {
            if kk.is_empty() {
                continue;
            }
            r.na_silnik.push((self.lista[i].format.clone(), kk.len()));
            self.lista[i].engine.adopt_baskets(kk);
        }
        r
    }

    /// Wszystkie koszyki wszystkich silników — do zrzutu i do panelu.
    pub fn koszyki(&self) -> Vec<Basket> {
        let mut v: Vec<Basket> = Vec::new();
        for s in &self.lista {
            v.extend(s.engine.baskets.iter().cloned());
        }
        v.sort_by_key(|b| b.id);
        v
    }
}

/// Co się stało z koszykami przy wznowieniu — do dziennika, nie do decyzji.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RaportPrzydzialu {
    /// przypisane po slocie z numeru koszyka (najmocniejszy dowód)
    pub po_slocie: usize,
    /// oddane silnikowi zapasowemu: numery sprzed wprowadzenia formatów
    /// albo koszyki formatu, który wypadł z aktywnego łańcucha
    pub na_zapas: Vec<u32>,
    /// nie było komu oddać — nie ma ANI JEDNEGO silnika
    pub porzucone: Vec<u32>,
    /// ile koszyków dostał który format
    pub na_silnik: Vec<(String, usize)>,
}

impl RaportPrzydzialu {
    pub fn opis(&self) -> String {
        let mut s = String::new();
        for (f, n) in &self.na_silnik {
            s.push_str(&format!("{f}: {n} koszyk(ów)\n"));
        }
        if !self.na_zapas.is_empty() {
            s.push_str(&format!(
                "⚠ Koszyki bez własnego slotu oddane silnikowi ZAPASOWEMU: {:?}\n\
                 To są albo numery sprzed wprowadzenia formatów (B1, B2, …), albo \
                 koszyki formatu, który wypadł z aktywnego łańcucha. Zarządza nimi \
                 preset silnika zapasowego, dopóki się nie domkną.\n",
                self.na_zapas
            ));
        }
        if !self.porzucone.is_empty() {
            s.push_str(&format!(
                "⛔ Koszyki BEZ OPIEKI (żaden format nie handluje): {:?}\n\
                 Pozycje zostają na rachunku z żywym ryzykiem i NIKT nimi nie zarządza.\n",
                self.porzucone
            ));
        }
        if s.is_empty() {
            s.push_str("nie było czego rozdzielać");
        }
        s
    }
}

impl Silniki {
    /// Wykonuje operację na silniku `i`, pokazując mu WYŁĄCZNIE jego pozycje.
    ///
    /// Jedyna droga, którą silnik dotyka brokera przy wielu formatach. Widok
    /// żyje tylko na czas domknięcia, a `Drop` oddaje schowane pozycje — więc
    /// nie da się zostawić rachunku w stanie połowicznym nawet nową gałęzią
    /// wyjścia dopisaną za pół roku.
    pub fn z_widokiem<B: Broker, R>(
        &mut self,
        i: usize,
        broker: &mut B,
        f: impl FnOnce(&mut Engine, &mut Widok<'_, B>) -> R,
    ) -> R {
        self.propagate_continuation_review();
        let kto = self.wlasnosc(i);
        // Rozbicie na pola, bo `poczekalnia` i `lista` muszą być pożyczone
        // jednocześnie, a przez `self` kompilator tego nie rozróżni.
        let Silniki {
            lista, poczekalnia, ..
        } = self;
        let result={
            let mut w = Widok::nowy(broker, kto, poczekalnia);
            f(&mut lista[i].engine, &mut w)
        };
        self.propagate_continuation_review();
        result
    }

    /// Account identity/unknown-owner faults propagate before another leg can
    /// add risk. A proved engine-owned intent fault remains local to its leg.
    fn propagate_continuation_review(&mut self) {
        let reason=self.lista.iter().find_map(|slot|slot.engine.continuation_review()
            .filter(|r|r.scope==crate::engine::ContinuationReviewScope::Account).map(|r|r.reason.clone()));
        if let Some(reason)=reason {
            for slot in &mut self.lista {
                slot.engine.hold_strategy_continuation(crate::engine::ContinuationReviewScope::Account,reason.clone());
            }
        }
    }

    /// Ile zamkniętych transakcji czeka jeszcze na właściciela.
    /// Trwale rosnąca liczba znaczy, że ktoś zamyka pozycje z koszyków,
    /// których żaden silnik nie uznaje za swoje — i to jest usterka.
    pub fn poczekalnia_len(&self) -> usize {
        self.poczekalnia.len()
    }

    /// Wykonuje operację na KAŻDYM silniku, każdemu pokazując wyłącznie jego.
    pub fn kazdy<B: Broker>(
        &mut self,
        broker: &mut B,
        mut f: impl FnMut(&mut Engine, &mut Widok<'_, B>),
    ) {
        for i in 0..self.lista.len() {
            self.z_widokiem(i, broker, |e, w| f(e, w));
        }
    }

    // ---------- agregaty do panelu, poczty i dziennika ----------
    //
    // Panel pokazuje JEDEN rachunek, więc liczby kilku silników trzeba złożyć
    // w jedną. Robimy to tutaj, w jednym miejscu, żeby nie powstały trzy
    // różne definicje „wyniku bota".

    /// Silnik zapasowy — reprezentant przy rzeczach, które dotyczą rachunku
    /// jako całości (dziennik odmów brokera, ustawienia rachunkowe).
    ///
    /// `lista[0]` jest tu bezpieczne, bo OBA konstruktory gwarantują listę
    /// niepustą: `pojedynczy` z definicji, a `zbuduj` dostawia silnik
    /// awaryjny, gdy żadna noga nie ma pliku presetu.
    pub fn glowny(&self) -> &Silnik {
        self.lista
            .iter()
            .find(|s| s.zapasowy)
            .unwrap_or(&self.lista[0])
    }

    pub fn glowny_mut(&mut self) -> &mut Silnik {
        let i = self.lista.iter().position(|s| s.zapasowy).unwrap_or(0);
        &mut self.lista[i]
    }

    /// Pierwsza blokada strażnika, jaką ktokolwiek ma. Panel pokazuje jedną
    /// kontrolkę „handel zatrzymany", a zatrzymany JEDEN format to już powód,
    /// żeby ją zapalić — inaczej użytkownik widziałby zielono przy koncie,
    /// które w połowie stoi.
    pub fn halted(&self) -> Option<String> {
        let zatrzymane: Vec<String> = self
            .lista
            .iter()
            .filter_map(|s| {
                s.engine
                    .halted
                    .as_ref()
                    .map(|r| format!("{}: {r}", s.format))
            })
            .collect();
        if zatrzymane.is_empty() {
            None
        } else if self.lista.len() == 1 {
            // przy jednym silniku nie doklejamy nazwy formatu — komunikat
            // trafia do panelu i ma brzmieć tak jak przed zmianą
            self.lista[0].engine.halted.clone()
        } else {
            Some(zatrzymane.join(" · "))
        }
    }

    /// Suma zrealizowanego wyniku dnia ze wszystkich formatów.
    pub fn realized_today(&self) -> f64 {
        self.lista
            .iter()
            .map(|s| s.engine.stats.realized_today)
            .sum()
    }

    /// Ile transakcji domknęło się dziś na całym rachunku.
    pub fn zamkniec_dzis(&self) -> usize {
        self.lista.iter().map(|s| s.engine.closed_today.len()).sum()
    }

    /// Wiadomości i sygnały policzone ze wszystkich silników.
    pub fn wiadomosci_i_sygnaly(&self) -> (u64, u64) {
        self.lista.iter().fold((0u64, 0u64), |(m, s), x| {
            (m + x.engine.stats.messages, s + x.engine.stats.signals)
        })
    }

    /// Żywe koszyki na całym rachunku.
    pub fn zywe_koszyki(&self) -> usize {
        self.lista
            .iter()
            .map(|s| s.engine.baskets.iter().filter(|b| b.alive()).count())
            .sum()
    }

    /// Najwyższy numer koszyka + 1 — do zrzutu na dysk. Przy wielu silnikach
    /// liczniki są rozłączne, więc jedna liczba nie odtworzy ich wszystkich;
    /// prawdziwe liczniki wracają z SLOTU, a ta wartość zostaje w pliku
    /// wyłącznie dla zgodności ze starszym formatem zrzutu.
    pub fn next_basket_id(&self) -> u32 {
        self.lista
            .iter()
            .map(|s| s.engine.next_basket_id())
            .max()
            .unwrap_or(1)
    }
}

// ============================================================
//  TESTY RDZENIA ROUTINGU
//
//  Komplet testów widoku brokera mieszka w `crates/app/src/routing.rs`
//  (na SimBrokerze). Tutaj testujemy wyłącznie to, co nie potrzebuje
//  symulatora: trasę, degradację pustej listy i obce obciążenie.
// ============================================================
#[cfg(test)]
mod testy {
    use super::*;
    use crate::broker::{BResult, BrokerError, OrderReq, PendingReq};
    use crate::types::{Account, CloseReason, ClosedTrade, Px, Quote, Ticket};

    /// Atrapa brokera do testów samego routingu — silniki tu nie handlują,
    /// więc wystarczy pusty rachunek i odmowa na każdą operację.
    struct Atrapa {
        poz: Vec<Position>,
        zle: Vec<PendingOrder>,
    }

    impl Atrapa {
        fn pusta() -> Self {
            Atrapa {
                poz: Vec::new(),
                zle: Vec::new(),
            }
        }
    }

    impl Broker for Atrapa {
        fn quote(&self) -> Quote {
            Quote {
                ts: 1_700_000_000_000,
                bid: 4000.0,
                ask: 4000.3,
            }
        }
        fn account(&self) -> Account {
            Account {
                balance: 1000.0,
                equity: 1000.0,
                margin: 0.0,
                free_margin: 1000.0,
                leverage: 500,
                credit: 0.0,
            }
        }
        fn stops_level(&self) -> f64 {
            0.0
        }
        fn positions(&self) -> &[Position] {
            &self.poz
        }
        fn pendings(&self) -> &[PendingOrder] {
            &self.zle
        }
        fn positions_mut(&mut self) -> &mut Vec<Position> {
            &mut self.poz
        }
        fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
            &mut self.zle
        }
        fn open_market(&mut self, _r: OrderReq) -> BResult<Ticket> {
            Err(BrokerError::Rejected)
        }
        fn place_pending(&mut self, _r: PendingReq) -> BResult<Ticket> {
            Err(BrokerError::Rejected)
        }
        fn modify_position(&mut self, _t: Ticket, _sl: Option<Px>, _tp: Option<Px>) -> BResult<()> {
            Err(BrokerError::Rejected)
        }
        fn modify_pending(
            &mut self,
            _t: Ticket,
            _p: Px,
            _sl: Option<Px>,
            _tp: Option<Px>,
        ) -> BResult<()> {
            Err(BrokerError::Rejected)
        }
        fn close_position(&mut self, _t: Ticket, _r: CloseReason) -> BResult<f64> {
            Err(BrokerError::Rejected)
        }
        fn close_partial(&mut self, _t: Ticket, _v: f64, _r: CloseReason) -> BResult<f64> {
            Err(BrokerError::Rejected)
        }
        fn cancel_pending(&mut self, _t: Ticket) -> BResult<()> {
            Err(BrokerError::Rejected)
        }
        fn drain_closed(&mut self) -> Vec<ClosedTrade> {
            Vec::new()
        }
    }

    fn lancuch_at_syn() -> Lancuch {
        let mut l = Lancuch {
            nazwa: "TEST".into(),
            ..Default::default()
        };
        l.presety.insert("ATFX".into(), "P-A".into());
        l.presety.insert("Synergy".into(), "P-S".into());
        l
    }

    fn presety() -> BTreeMap<String, Settings> {
        let mut m = BTreeMap::new();
        m.insert("P-A".to_string(), Settings::default());
        m.insert("P-S".to_string(), Settings::default());
        m
    }

    /// Nazwa formatu w konfiguracji kanału jest wpisywana ręcznie — „ atfx ”
    /// musi trafić do silnika „ATFX”, a nie umrzeć jako FormatNieHandluje
    /// (audyt routing.rs:553: kanał z inną wielkością liter dostawał silnik,
    /// ale każdy sygnał odpadał).
    #[test]
    fn trasa_wybacza_wielkosc_liter_i_spacje_brzegowe() {
        let (s, braki) =
            Silniki::zbuduj(&lancuch_at_syn(), &presety(), &Settings::default(), 1000.0);
        assert!(braki.is_empty());
        let src = SourceKey::new(-1001, None);

        let wprost = s.trasa(&src, Some("ATFX".into())).unwrap();
        assert_eq!(s.trasa(&src, Some(" atfx ".into())).unwrap(), wprost);
        assert_eq!(
            s.trasa(&src, Some("SYNERGY".into())).unwrap(),
            s.indeks_formatu("Synergy").unwrap()
        );
        // same spacje to BRAK formatu, nie format o nazwie ze spacji
        assert!(matches!(
            s.trasa(&src, Some("   ".into())),
            Err(BrakTrasy::KanalBezFormatu { .. })
        ));
        // nieznany format dalej odpada z właściwym powodem
        assert!(matches!(
            s.trasa(&src, Some("ZEN".into())),
            Err(BrakTrasy::FormatNieHandluje { .. })
        ));
    }

    /// Łańcuch, którego żadna noga nie ma pliku presetu, dawał pustą listę,
    /// a `glowny()` indeksuje `lista[0]` — panika pętli żywej zamiast
    /// degradacji (audyt routing.rs:481+817).
    #[test]
    fn lancuch_bez_zadnego_presetu_dostaje_silnik_awaryjny_zamiast_paniki() {
        // obie nogi wskazują pliki, których nie ma na dysku
        let (s, braki) = Silniki::zbuduj(
            &lancuch_at_syn(),
            &BTreeMap::new(),
            &Settings::default(),
            1000.0,
        );
        assert_eq!(braki.len(), 2, "każda noga bez pliku musi zostać zgłoszona");
        assert_eq!(
            s.lista.len(),
            1,
            "zamiast pustej listy — jeden silnik awaryjny"
        );

        let g = s.glowny(); // przed naprawą: panika na `lista[0]` pustego wektora
        assert!(
            g.zapasowy,
            "awaryjny musi przygarniać zastane koszyki i ręczne bilety"
        );
        assert!(
            g.tylko_zarzadzanie,
            "awaryjny nie może brać nowych sygnałów"
        );
        assert!(
            !g.powod.is_empty(),
            "panel musi umieć powiedzieć, czemu nic nie handluje"
        );
        assert_eq!(g.slot, wielosilnik::SLOT_STARY);
        // znany format kanału też nie wchodzi — silnika ATFX po prostu nie ma
        assert!(s
            .trasa(&SourceKey::new(-1001, None), Some("ATFX".into()))
            .is_err());

        // łańcuch bez ANI JEDNEJ nogi — ta sama degradacja, bez braków
        let pusty = Lancuch {
            nazwa: "PUSTY".into(),
            ..Default::default()
        };
        let (s2, braki2) = Silniki::zbuduj(&pusty, &presety(), &Settings::default(), 1000.0);
        assert!(braki2.is_empty());
        assert_eq!(s2.lista.len(), 1);
        assert!(s2.glowny().tylko_zarzadzanie);
    }

    /// Wynik dnia nogi w obcym obciążeniu ma być ZREALIZOWANY wynik tej nogi,
    /// a nie dobowa delta equity całego konta: equity jest jedno, więc każda
    /// z N nóg wnosiła pełną deltę konta i pułapy dobowe łańcucha odpalały
    /// przy 1/N progu (audyt routing.rs:608, TOP 9).
    #[test]
    fn przelicz_obce_liczy_zrealizowane_nogi_a_nie_delte_equity_konta() {
        let (mut s, _) =
            Silniki::zbuduj(&lancuch_at_syn(), &presety(), &Settings::default(), 1000.0);
        let ia = s.indeks_formatu("ATFX").unwrap();
        let ib = s.indeks_formatu("Synergy").unwrap();

        // equity KONTA jest jedno — oba silniki widzą tę samą dobową deltę +100
        for i in [ia, ib] {
            s.lista[i].engine.stats.day_start_equity = 1000.0;
            s.lista[i].engine.stats.equity = 1100.0;
        }
        s.lista[ia].engine.stats.realized_today = 30.0;
        s.lista[ib].engine.stats.realized_today = 50.0;

        let b = Atrapa::pusta();
        s.przelicz_obce(&b, None);

        // przed naprawą każda noga dostawała tu 100 $ (cudzą „deltę konta”)
        assert_eq!(s.lista[ia].engine.obce.zrealizowane_dzis, 50.0);
        assert_eq!(s.lista[ib].engine.obce.zrealizowane_dzis, 30.0);
    }
}
