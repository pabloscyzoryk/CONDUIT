
use crate::oczy::Oczy;
use crate::rama::{GeometriaPomyslu, Strona, Ts};

/// Jedna otwarta pozycja widziana przez mózg.
#[derive(Debug, Clone, Copy)]
pub struct Szczebel {
    pub ticket: u64,
    pub strona: Strona,
    pub cena_wejscia: f64,
    pub wolumen: f64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
    /// GŁĘBOKOŚĆ: 0 = najpłytszy (najgorsza cena wejścia), rosnąco w głąb.
    /// Dla BUY głębszy znaczy TAŃSZY, dla SELL — DROŻSZY.
    pub glebokosc: u16,
    pub ts_otwarcia: Ts,
    /// bieżący wynik w dolarach, po kosztach znanych w tej chwili
    pub wynik_usd: f64,
    /// najlepszy wynik, jaki ta pozycja OSIĄGNĘŁA (szczyt korzystnego wychylenia)
    pub szczyt_usd: f64,
    /// najgorszy wynik, jaki ta pozycja PRZETRWAŁA
    pub dno_usd: f64,
}

impl Szczebel {
    /// Czy stop stoi tak, że pozycja nie może już stracić (z zapasem `zapas`).
    ///
    /// To jest pytanie „czy ta noga jest już bezpieczna", a nie „czy stop jest
    /// na wejściu" — bo stop na wejściu przy niezerowym spreadzie i swapie
    /// bywa minimalnie stratny, co ta funkcja uczciwie pokazuje.
    pub fn zabezpieczony(&self, zapas: f64) -> bool {
        match (self.sl, self.strona) {
            (Some(sl), Strona::Buy) => sl >= self.cena_wejscia + zapas,
            (Some(sl), Strona::Sell) => sl <= self.cena_wejscia - zapas,
            (None, _) => false,
        }
    }

    /// Ile z osiągniętego szczytu zysku pozycja już oddała, w ułamku.
    /// `NaN`, gdy nigdy nie była na plusie — „oddała 0 %" byłoby wtedy
    /// liczbą udającą wiedzę.
    pub fn oddane_ze_szczytu(&self) -> f64 {
        if self.szczyt_usd <= 0.0 {
            return f64::NAN;
        }
        (self.szczyt_usd - self.wynik_usd) / self.szczyt_usd
    }
}

/// Koszyk — jeden pomysł z kanału, wiele szczebli.
#[derive(Debug, Clone)]
pub struct Koszyk {
    pub id: u32,
    pub rama_id: u32,
    pub geometria: GeometriaPomyslu,
    pub ts_zawiazania: Ts,
    /// szczeble POSORTOWANE po głębokości rosnąco (0 = najpłytszy)
    pub szczeble: Vec<Szczebel>,
    /// ile zleceń oczekujących jeszcze czeka na wypełnienie
    pub oczekujacych: u16,
    /// najdalszy osiągnięty etap celu (0 = żaden, 1 = TP1, …)
    pub etap_celu: u8,
    /// czy kanał ogłosił już RISK FREE dla tego pomysłu
    pub rf_ogloszony: bool,
    /// ile dolarów ta rama już wydała z budżetu ryzyka
    pub budzet_wydany_usd: f64,
}

impl Koszyk {
    pub fn wynik_usd(&self) -> f64 {
        self.szczeble.iter().map(|s| s.wynik_usd).sum()
    }
    pub fn wolumen(&self) -> f64 {
        self.szczeble.iter().map(|s| s.wolumen).sum()
    }
    /// Najlepszy szczebel — ten, który kanał zostawiłby przy RISK FREE.
    /// „Najlepszy" znaczy o największym wyniku, a nie najgłębszy: przy luce
    /// otwarcia najgłębszy potrafi być NAJGORSZY, a reguła ma opisywać
    /// intencję („zostaw ten, który zarabia"), nie geometrię.
    pub fn najlepszy(&self) -> Option<&Szczebel> {
        self.szczeble.iter().max_by(|a, b| {
            a.wynik_usd
                .partial_cmp(&b.wynik_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
    /// Szczeble, które kanał zainkasowałby przy RISK FREE: wszystkie poza
    /// najlepszym I będące na plusie. Zamykanie stratnych „bo tak każe
    /// komunikat" to realizacja straty, której kanał nie ogłasza.
    pub fn do_inkasa(&self) -> Vec<&Szczebel> {
        let n = self.najlepszy().map(|x| x.ticket);
        self.szczeble
            .iter()
            .filter(|s| Some(s.ticket) != n && s.wynik_usd > 0.0)
            .collect()
    }
}

/// Stan rachunku — jedyne, co mózg wie o pieniądzach.
#[derive(Debug, Clone, Copy)]
pub struct Rachunek {
    pub saldo: f64,
    pub equity: f64,
    pub margines_uzyty: f64,
    pub margines_wolny: f64,
    /// poziom marginesu w procentach; `None` = brak ekspozycji
    pub poziom_marginesu: Option<f64>,
    pub dzwignia: f64,
    /// wynik dnia bieżącego, w dolarach
    pub wynik_dnia_usd: f64,
    /// ile stopów z rzędu padło — pamięć krótka, ale bywa rozstrzygająca
    pub seria_stopow: u16,
}

impl Rachunek {
    pub fn poziom(&self) -> f64 {
        self.poziom_marginesu.unwrap_or(f64::INFINITY)
    }
}

/// Komplet wiedzy mózgu w JEDNEJ chwili.
///
/// Nieruchomy: powstaje raz na puls i nie zmienia się w trakcie decyzji.
/// Dzięki temu dwa wywołania `decyduj` na tym samym wejściu MUSZĄ dać ten sam
/// wynik — a to jest jedyny sposób, żeby cień mógł cokolwiek udowodnić.
#[derive(Debug, Clone)]
pub struct Wejscie<'a> {
    pub ts: Ts,
    pub oczy: &'a Oczy,
    pub rachunek: Rachunek,
    /// koszyki żywe w tej chwili
    pub koszyki: &'a [Koszyk],
    /// który koszyk jest przedmiotem tej decyzji (`None` = decyzja portfelowa)
    pub koszyk: Option<usize>,
}

impl<'a> Wejscie<'a> {
    pub fn biezacy(&self) -> Option<&Koszyk> {
        self.koszyk.and_then(|i| self.koszyki.get(i))
    }
    /// Łączna ekspozycja w lotach — do decyzji portfelowych.
    pub fn wolumen_lacznie(&self) -> f64 {
        self.koszyki.iter().map(|k| k.wolumen()).sum()
    }
    /// Ile koszyków jest pod wodą. Jeden stratny koszyk to normalna praca;
    /// pięć naraz to inna sytuacja i polityka ma prawo je rozróżnić.
    pub fn koszykow_pod_woda(&self) -> usize {
        self.koszyki.iter().filter(|k| k.wynik_usd() < 0.0).count()
    }
}
