
use crate::rama::Ts;

/// Okresy, w których mózg widzi rynek. Kolejność jest częścią kontraktu:
/// indeks w tablicy [`Oczy::okna`] odpowiada pozycji w tym wykazie.
///
/// Tik jest osobno (nie jest świecą) i mieszka w [`Oczy::tik`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Okres {
    M1,
    M5,
    M15,
    H1,
    H4,
    D1,
    W1,
    MN1,
}

impl Okres {
    pub const WSZYSTKIE: [Okres; 8] = [
        Okres::M1,
        Okres::M5,
        Okres::M15,
        Okres::H1,
        Okres::H4,
        Okres::D1,
        Okres::W1,
        Okres::MN1,
    ];

    /// Długość świecy w milisekundach.
    ///
    /// Tydzień i miesiąc są PRZYBLIŻONE (7 dni i 30 dni), i to jest świadome:
    /// dokładne granice kalendarzowe wymagałyby strefy czasowej brokera i
    /// reguł przejścia na czas letni, czyli dwóch źródeł prawdy o czasie
    /// w kodzie, który ma być deterministyczny. Do mierzenia „jak daleko
    /// jesteśmy w ruchu tygodniowym" stała długość wystarcza; gdyby kiedyś
    /// przestała, ma to być JEDNA zmiana tutaj, a nie poprawka w politykach.
    pub const fn ms(self) -> i64 {
        match self {
            Okres::M1 => 60_000,
            Okres::M5 => 300_000,
            Okres::M15 => 900_000,
            Okres::H1 => 3_600_000,
            Okres::H4 => 14_400_000,
            Okres::D1 => 86_400_000,
            Okres::W1 => 604_800_000,
            Okres::MN1 => 2_592_000_000,
        }
    }

    pub const fn nazwa(self) -> &'static str {
        match self {
            Okres::M1 => "1m",
            Okres::M5 => "5m",
            Okres::M15 => "15m",
            Okres::H1 => "1h",
            Okres::H4 => "4h",
            Okres::D1 => "1D",
            Okres::W1 => "1W",
            Okres::MN1 => "1M",
        }
    }
}

/// Jedna świeca. `domknieta = false` znaczy „ta świeca jeszcze trwa".
#[derive(Debug, Clone, Copy, Default)]
pub struct Swieca {
    pub start_ms: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    /// liczba tików — nasz zamiennik wolumenu, bo strumień bid/ask go nie niesie
    pub tikow: u32,
    pub domknieta: bool,
}

impl Swieca {
    #[inline]
    fn nowa(start_ms: i64, px: f64) -> Self {
        Swieca {
            start_ms,
            o: px,
            h: px,
            l: px,
            c: px,
            tikow: 1,
            domknieta: false,
        }
    }
    #[inline]
    fn dolóż(&mut self, px: f64) {
        if px > self.h {
            self.h = px;
        }
        if px < self.l {
            self.l = px;
        }
        self.c = px;
        self.tikow = self.tikow.saturating_add(1);
    }
    #[inline]
    pub fn zakres(&self) -> f64 {
        self.h - self.l
    }
    /// Położenie zamknięcia w zakresie świecy: 0 = na dnie, 1 = na szczycie.
    /// Świeca bez zakresu (jedna cena) daje 0,5 — środek, bo ani góra, ani dół
    /// nie jest wtedy prawdą.
    #[inline]
    pub fn polozenie(&self) -> f64 {
        let z = self.zakres();
        if z <= 0.0 {
            0.5
        } else {
            (self.c - self.l) / z
        }
    }
}

/// Ile świec wstecz pamiętamy na każdym okresie.
///
/// Dwadzieścia to kompromis: wystarcza na zakres, tempo i położenie względem
/// niedawnej historii, a nie zamienia ekstraktora w bazę danych. Przy ośmiu
/// okresach daje 160 świec — stała, mała, mieszcząca się w pamięci podręcznej
/// procesora, co jest tu ważniejsze niż długość historii.
pub const PAMIEC: usize = 20;

/// Jeden okres: świeca bieżąca plus pierścień domkniętych.
#[derive(Debug, Clone)]
pub struct Okno {
    pub okres: Okres,
    pub biezaca: Option<Swieca>,
    /// pierścień domkniętych, `hist[0]` = ostatnia domknięta
    hist: [Swieca; PAMIEC],
    ile: usize,
}

impl Okno {
    fn nowe(okres: Okres) -> Self {
        Okno {
            okres,
            biezaca: None,
            hist: [Swieca::default(); PAMIEC],
            ile: 0,
        }
    }

    #[inline]
    fn tik(&mut self, ts: i64, px: f64) {
        let dl = self.okres.ms();
        // Kotwiczenie na wielokrotności długości, a NIE na pierwszym tiku:
        // inaczej granice świec zależałyby od tego, kiedy uruchomiono bota,
        // i dwa przebiegi na tych samych danych dałyby inne świece.
        let start = ts - ts.rem_euclid(dl);
        match &mut self.biezaca {
            Some(s) if s.start_ms == start => s.dolóż(px),
            Some(s) => {
                let mut zamknieta = *s;
                zamknieta.domknieta = true;
                // przesunięcie pierścienia: hist[0] to zawsze NAJŚWIEŻSZA
                for i in (1..PAMIEC).rev() {
                    self.hist[i] = self.hist[i - 1];
                }
                self.hist[0] = zamknieta;
                self.ile = (self.ile + 1).min(PAMIEC);
                self.biezaca = Some(Swieca::nowa(start, px));
            }
            None => self.biezaca = Some(Swieca::nowa(start, px)),
        }
    }

    /// Domknięte świece, od najświeższej. Świecy BIEŻĄCEJ tu nie ma —
    /// to jest granica między „co się stało" a „co się właśnie dzieje".
    pub fn domkniete(&self) -> &[Swieca] {
        &self.hist[..self.ile]
    }

    /// Średni zakres N ostatnich DOMKNIĘTYCH świec (zamiennik ATR, liczony
    /// zakresem, nie kwadratami zwrotów — ta sama konwencja co `atr_proxy`
    /// w silniku, żeby dwie części projektu nie mierzyły zmienności inaczej).
    pub fn sredni_zakres(&self, n: usize) -> f64 {
        let s = self.domkniete();
        let k = n.min(s.len());
        if k == 0 {
            return f64::NAN;
        }
        s[..k].iter().map(|x| x.zakres()).sum::<f64>() / k as f64
    }

    /// Położenie ceny w zakresie N ostatnich domkniętych świec: 0 = przy dnie,
    /// 1 = przy szczycie. To jest „gdzie jesteśmy" bez zakładania trendu.
    pub fn polozenie_w_zakresie(&self, n: usize, px: f64) -> f64 {
        let s = self.domkniete();
        let k = n.min(s.len());
        if k == 0 {
            return f64::NAN;
        }
        let mut hi = f64::MIN;
        let mut lo = f64::MAX;
        for x in &s[..k] {
            if x.h > hi {
                hi = x.h;
            }
            if x.l < lo {
                lo = x.l;
            }
        }
        if hi <= lo {
            0.5
        } else {
            (px - lo) / (hi - lo)
        }
    }

    /// SWING HIGH — ostatni potwierdzony szczyt struktury (opór).
    ///
    /// Fraktal o promieniu `r`: świeca, której maksimum jest wyższe niż `r`
    /// świec z każdej strony. „Potwierdzony" znaczy, że po niej padło już `r`
    /// świec — bez tego warunku szczyt byłby wykrywany zanim się skończył,
    /// czyli z wiedzą o przyszłości.
    ///
    /// Zwraca `NaN`, gdy w pamięci nie ma jeszcze potwierdzonego swingu.
    /// To NIE jest to samo co „brak oporu" i polityka musi te dwie rzeczy
    /// rozróżniać — dlatego NaN, a nie zero ani ostatnie maksimum.
    pub fn swing_high(&self, r: usize) -> f64 {
        let h = self.domkniete();
        if h.len() < 2 * r + 1 {
            return f64::NAN;
        }
        // hist[0] jest NAJŚWIEŻSZA, więc świeca środkowa ma indeks i,
        // a „po niej" to indeksy MNIEJSZE.
        for i in r..(h.len() - r) {
            let c = h[i].h;
            let lewo = (1..=r).all(|k| h[i + k].h <= c);
            let prawo = (1..=r).all(|k| h[i - k].h <= c);
            if lewo && prawo {
                return c;
            }
        }
        f64::NAN
    }

    /// SWING LOW — ostatni potwierdzony dołek struktury (wsparcie).
    pub fn swing_low(&self, r: usize) -> f64 {
        let h = self.domkniete();
        if h.len() < 2 * r + 1 {
            return f64::NAN;
        }
        for i in r..(h.len() - r) {
            let c = h[i].l;
            let lewo = (1..=r).all(|k| h[i + k].l >= c);
            let prawo = (1..=r).all(|k| h[i - k].l >= c);
            if lewo && prawo {
                return c;
            }
        }
        f64::NAN
    }

    /// TREND: nachylenie zamknięć N ostatnich domkniętych świec, znormalizowane
    /// średnim zakresem. Wynik w jednostkach „ile zakresów świecy na świecę".
    ///
    /// Regresja liniowa, a nie różnica pierwszej i ostatniej: różnica opisuje
    /// dwa punkty, a nachylenie — cały odcinek. Świeca-szpilka na końcu okna
    /// przewraca różnicę, a nachylenia prawie nie rusza.
    pub fn trend(&self, n: usize) -> f64 {
        let h = self.domkniete();
        let k = n.min(h.len());
        if k < 3 {
            return f64::NAN;
        }
        // x liczymy OD NAJSTARSZEJ: hist jest odwrócone, więc x = k-1-i
        let (mut sx, mut sy, mut sxy, mut sxx) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..k {
            let x = (k - 1 - i) as f64;
            let y = h[i].c;
            sx += x;
            sy += y;
            sxy += x * y;
            sxx += x * x;
        }
        let kk = k as f64;
        let m = kk * sxx - sx * sx;
        if m.abs() < 1e-12 {
            return f64::NAN;
        }
        let nachylenie = (kk * sxy - sx * sy) / m;
        let skala = self.sredni_zakres(k);
        if !skala.is_finite() || skala <= 0.0 {
            return f64::NAN;
        }
        nachylenie / skala
    }

    /// SERIA: ile ostatnich domkniętych świec ma ten sam kierunek.
    /// Dodatnia dla wzrostowych, ujemna dla spadkowych.
    ///
    /// Seria jest miarą JEDNOSTAJNOŚCI ruchu, której nachylenie nie oddaje:
    /// pięć świec po trochu w górę i jedna wielka dają to samo nachylenie,
    /// a to są dwie różne sytuacje dla pozycji, która w nich siedzi.
    pub fn seria(&self) -> i32 {
        let h = self.domkniete();
        if h.is_empty() {
            return 0;
        }
        let w_gore = h[0].c >= h[0].o;
        let mut n = 0i32;
        for s in h {
            if (s.c >= s.o) != w_gore {
                break;
            }
            n += 1;
        }
        if w_gore {
            n
        } else {
            -n
        }
    }

    /// POPRZEDNIA ŚWIECA tego okresu — dla D1 to wczorajsze maksimum i
    /// minimum, dla W1 zeszłotygodniowe.
    ///
    /// To jest najstarszy i najpowszechniejszy poziom odniesienia w handlu:
    /// wczorajszy szczyt i dołek widzi każdy uczestnik rynku, więc cena
    /// zachowuje się przy nich inaczej niż w przypadkowym miejscu. W katalogu
    /// wskaźników odpowiada mu `DailyHighLow`.
    ///
    /// Bierzemy `hist[0]`, czyli ostatnią DOMKNIĘTĄ — świeca bieżąca jest
    /// jeszcze w trakcie i jej maksimum może się zmienić.
    pub fn poprzednia(&self) -> Option<Swieca> {
        self.domkniete().first().copied()
    }

    pub fn rezim(&self, n: usize) -> f64 {
        let a = self.sredni_zakres(n);
        let b = self.sredni_zakres(n * 3);
        if !a.is_finite() || !b.is_finite() || b <= 0.0 {
            return f64::NAN;
        }
        a / b
    }

    /// ODCHYLENIE od średniej zamknięć N świec, w jednostkach zakresu.
    ///
    /// Rdzeń wszystkich kopert (Nadaraya-Watson, Kijun-Sen, Bollinger):
    /// „jak daleko cena odeszła od tego, wokół czego się kręci". Dodatnie
    /// znaczy powyżej średniej — kierunek NIE jest tu odbijany dla SELL, bo
    /// odbicie robi warstwa cech, żeby to samo pytanie nie miało dwóch
    /// odpowiedzi w dwóch miejscach.
    pub fn odchylenie(&self, n: usize, px: f64) -> f64 {
        let h = self.domkniete();
        let k = n.min(h.len());
        if k < 2 {
            return f64::NAN;
        }
        let sr = h[..k].iter().map(|x| x.c).sum::<f64>() / k as f64;
        let sk = self.sredni_zakres(k);
        if !sk.is_finite() || sk <= 0.0 {
            return f64::NAN;
        }
        (px - sr) / sk
    }

    /// BLOK ZLECEŃ: ostatnia świeca PRZECIWNA przed impulsem w naszą stronę.
    ///
    /// Wzorzec z `OrderBlock.mq5` i z całej rodziny „smart money": zanim rynek
    /// ruszy w górę, zwykle powstaje ostatnia świeca spadkowa — i to jej
    /// zakres bywa poziomem, na którym cena wraca. Szukamy jej wśród
    /// domkniętych i zwracamy jej dolną krawędź (dla `w_gore`) albo górną.
    ///
    /// Impuls definiujemy skromnie: świeca o zakresie większym niż dwukrotność
    /// średniej. To jest jedyna liczba w tej funkcji i jest tu dlatego, że
    /// „impuls" bez progu nie ma definicji — a nie dlatego, że dwójka jest
    /// wyjątkowa.
    pub fn blok_zlecen(&self, w_gore: bool) -> f64 {
        let h = self.domkniete();
        if h.len() < 4 {
            return f64::NAN;
        }
        let sr = self.sredni_zakres(10);
        if !sr.is_finite() || sr <= 0.0 {
            return f64::NAN;
        }
        // idziemy od najświeższej: szukamy impulsu, potem świecy przeciwnej
        // BEZPOŚREDNIO przed nim (czyli o indeks WIĘKSZY, bo hist jest odwrócone)
        for i in 0..h.len().saturating_sub(1) {
            let s = h[i];
            let impuls = s.zakres() > 2.0 * sr && ((s.c >= s.o) == w_gore);
            if !impuls {
                continue;
            }
            let p = h[i + 1];
            if (p.c >= p.o) != w_gore {
                return if w_gore { p.l } else { p.h };
            }
        }
        f64::NAN
    }

    /// Zmiana ceny przez N ostatnich domkniętych świec, w cenie.
    pub fn zmiana(&self, n: usize) -> f64 {
        let s = self.domkniete();
        let k = n.min(s.len());
        if k == 0 {
            return f64::NAN;
        }
        s[0].c - s[k - 1].o
    }
}

/// Obraz rynku widziany przez mózg w JEDNEJ chwili.
///
/// Aktualizowany przyrostowo, jeden tik na raz. Nie zna brokera, pozycji ani
/// ustawień — to jest surowy widok świata, a nie widok naszego rachunku.
#[derive(Debug, Clone)]
pub struct Oczy {
    /// ostatni tik: czas, bid, ask
    pub tik: Option<(Ts, f64, f64)>,
    /// poprzedni tik — do tempa i do wykrycia przerwy w strumieniu
    pub tik_poprz: Option<(Ts, f64, f64)>,
    pub okna: [Okno; 8],
    /// ile tików wpadło od początku (do sprawdzenia, czy obraz jest już pełny)
    pub tikow: u64,
    pub max_przerwa_ms: i64,
}

impl Default for Oczy {
    fn default() -> Self {
        Self::nowe()
    }
}

impl Oczy {
    pub fn nowe() -> Self {
        Oczy {
            tik: None,
            tik_poprz: None,
            okna: [
                Okno::nowe(Okres::M1),
                Okno::nowe(Okres::M5),
                Okno::nowe(Okres::M15),
                Okno::nowe(Okres::H1),
                Okno::nowe(Okres::H4),
                Okno::nowe(Okres::D1),
                Okno::nowe(Okres::W1),
                Okno::nowe(Okres::MN1),
            ],
            tikow: 0,
            max_przerwa_ms: 0,
        }
    }

    /// Jeden tik. To jest CAŁA ścieżka zapisu tego modułu.
    ///
    /// Cena świec to MID — bo świeca ma opisywać rynek, a nie stronę, po
    /// której akurat handlujemy. Spread jest osobno w [`Oczy::spread`], gdzie
    /// polityka może go zobaczyć jako koszt, a nie pomylić z ruchem.
    #[inline]
    pub fn tik(&mut self, ts: Ts, bid: f64, ask: f64) {
        if let Some((p, _, _)) = self.tik {
            let d = ts - p;
            if d > self.max_przerwa_ms {
                self.max_przerwa_ms = d;
            }
        }
        self.tik_poprz = self.tik;
        self.tik = Some((ts, bid, ask));
        self.tikow += 1;
        let mid = (bid + ask) * 0.5;
        for o in self.okna.iter_mut() {
            o.tik(ts, mid);
        }
    }

    #[inline]
    pub fn okno(&self, o: Okres) -> &Okno {
        &self.okna[Okres::WSZYSTKIE.iter().position(|x| *x == o).unwrap_or(0)]
    }

    pub fn wlasny_kierunek(&self) -> f64 {
        // Wagi rosną z horyzontem: ruch dzienny mówi więcej o tym, dokąd
        // rynek zmierza, niż pięciominutowy. Suma wag = 1, żeby wynik
        // pozostał w przedziale −1…+1 bez dodatkowego dzielenia.
        const SKLAD: [(Okres, f64, usize); 5] = [
            (Okres::M5, 0.10, 8),
            (Okres::M15, 0.15, 8),
            (Okres::H1, 0.25, 8),
            (Okres::H4, 0.25, 8),
            (Okres::D1, 0.25, 5),
        ];
        let mut suma = 0.0;
        let mut waga = 0.0;
        for (ok, w, n) in SKLAD {
            let okno = self.okno(ok);
            let t = okno.trend(n);
            if !t.is_finite() {
                continue;
            }
            // Nachylenie ścinamy do ±1: interesuje nas KIERUNEK i jego siła,
            // a nie to, że jeden okres akurat pędzi dziesięć razy szybciej.
            suma += w * t.clamp(-1.0, 1.0);
            waga += w;
        }
        if waga <= 0.0 {
            return 0.0;
        }
        suma / waga
    }

    #[inline]
    pub fn mid(&self) -> f64 {
        self.tik.map(|(_, b, a)| (b + a) * 0.5).unwrap_or(f64::NAN)
    }

    #[inline]
    pub fn spread(&self) -> f64 {
        self.tik.map(|(_, b, a)| a - b).unwrap_or(f64::NAN)
    }

    /// Odstęp od poprzedniego tiku w ms. Duża wartość znaczy albo ciszę
    /// rynku, albo zerwany kanał — i mózg MUSI je rozróżniać, bo pierwsze
    /// jest normalne, a drugie znaczy, że pozycje są bez opieki.
    #[inline]
    pub fn odstep_ms(&self) -> i64 {
        match (self.tik, self.tik_poprz) {
            (Some((t, _, _)), Some((p, _, _))) => t - p,
            _ => 0,
        }
    }
}
