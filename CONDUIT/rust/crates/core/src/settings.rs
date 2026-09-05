//! Konfiguracja silnika.
//!
//! Zasada naczelna: **każda akcja z kanału ma wiele wariantów zachowania**.
//! RISK FREE, OUT AT ENTRY, TP HIT, SL HIT czy CANCEL nie są zaszyte na sztywno
//! — to punkty konfiguracji, bo dokładne znaczenie zależy od kanału, od tego jak
//! prowadzi go dany sygnalista, i od tego czego chcemy od koszyka.

use serde::{Deserialize, Serialize};

// ============================================================
//  WARIANTY REAKCJI NA KOMUNIKATY
// ============================================================

/// Co zrobić na „RISK FREE".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFreeMode {
    /// Semantyka ATFX: zamknij CAŁY koszyk po rynku (zyski głębokich wejść
    /// kompensują straty płytkich), zostaw `runners` pozycji NAJBLIŻSZYCH
    /// poziomowi z komunikatu, ich SL na breakeven.
    CloseAllKeepNearest,
    /// Zostaw N NAJLEPSZYCH (największy zysk), resztę zamknij.
    CloseAllKeepBest,
    /// Zamknij tylko te, które są na plusie; stratne zostaw z SL bez zmian.
    CloseProfitableOnly,
    /// Nic nie zamykaj — tylko przesuń SL wszystkich na breakeven.
    MoveSlToBeOnly,
    /// Zamknij wszystko bez wyjątku.
    CloseEverything,
    /// Ignoruj komunikat.
    Ignore,
}

/// Co runner dostaje po RISK FREE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFreeRunnerTarget {
    /// zostaje przy dotychczasowym celu
    KeepTp,
    /// dostaje najdalszy cel z drabinki
    LastTp,
    /// bez TP — prowadzony wyłącznie trailingiem
    NoTpTrailOnly,
    /// kolejny nietrafiony cel
    NextTp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SppSlMode {
    Off,
    /// przestaw stop CAŁEGO koszyka na podany poziom
    Stop,
    /// przestaw tylko, gdy poziom jest KORZYSTNIEJSZY (ciaśniejszy) od
    /// dzisiejszego stopu pozycji — nigdy nie pogarszaj
    OnlyIfBetter,
    /// zastosuj wyłącznie do RUNNERÓW (pozycji bez własnego celu); warstwa
    /// bankująca zostaje ze swoim stopem
    RunnersOnly,
    /// część wspólna dwóch poprzednich: runnery, i tylko gdy poziom jest
    /// korzystniejszy. Wariant dołożony dlatego, że oba ograniczenia są
    /// niezależne — jeżeli którekolwiek z nich ratuje regułę, to ich
    /// przecięcie jest naturalnym kandydatem, a nie kolejnym pokrętłem.
    RunnersOnlyIfBetter,
    BankersOnly,
}

/// Co zrobić na „OUT AT ENTRY".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutAtEntryMode {
    /// zamknij wszystkie pozycje koszyka po rynku
    CloseAll,
    /// zamknij tylko stratne
    CloseLosersOnly,
    /// zamknij tylko te ~na zero (w paśmie `oae_band_pts`)
    CloseFlatOnly,
    /// nie zamykaj — przesuń SL na wejście
    MoveSlToBe,
    Ignore,
}

fn oae_pod_woda_domyslna() -> OaePodWoda {
    OaePodWoda::NicNieRob
}

fn runner_krok_domyslny() -> f64 {
    10.0
}
fn sr_tf_min_domyslne() -> u32 {
    1
}
fn sr_fractal_domyslny() -> u32 {
    3
}
fn sr_offset_domyslny() -> f64 {
    0.5
}
fn sr_min_dist_tp_domyslny() -> f64 {
    2.0
}
fn sr_okno_h_domyslne() -> u32 {
    24
}
fn sr_atr_period_domyslny() -> u32 {
    14
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OaePodWoda {
    /// jak dotąd — breakeven nie do postawienia, więc nic
    NicNieRob,
    /// zamknij tę pozycję po rynku (kanał uznał pomysł za skończony)
    Zamknij,
    /// dociągnij stop TAK BLISKO wejścia, jak pozwala broker
    DociagnijStop,
}

/// Co zrobić na „SL HIT" z kanału.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlHitMode {
    /// skasuj niezafillowane limity, pozycje zostaw na własnym SL
    CancelPendings,
    /// zamknij wszystko po rynku
    CloseAll,
    /// zweryfikuj ceną — jeśli rynek jest daleko po stronie zysku, zignoruj
    VerifyByPrice,
    Ignore,
}

/// Kiedy kasować niezafillowane limity koszyka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingLifetime {
    /// do pierwszego trafionego celu (domyślne zachowanie ATFX)
    UntilTp1,
    UntilTp2,
    UntilTp3,
    /// żyją aż do końca koszyka
    Never,
}

/// Harmonogram bankowania zysku na kolejnych celach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TpSchedule {
    /// każda pozycja dostaje najdalszy cel, SL trailuje
    AllRunners,
    /// zamykaj procent pozycji na każdym celu
    ScaleOutPct,
    /// harmonogram procentowy 15/30/30/20
    OfficialPct,
    /// harmonogram liczbowy, np. "1,1,1"
    OfficialCounts,
    /// kaskada: najgorsze wejścia dostają TP1, kolejne TP2…
    Ladder,
    /// wszystko na TP1 (najszybszy zysk)
    AllAtTp1,
}

/// Tryb podążania SL za zyskiem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailMode {
    /// SL o stałą lukę za ceną
    Gap,
    /// SL blokuje % szczytu zysku
    LockPct,
    /// drabinka progów „zysk:blokada"
    Tiered,
    /// SL o lukę `trail_atr_mult × ATR` za BIEŻĄCĄ ceną wyjścia.
    ///
    /// To samo, co [`TrailMode::Gap`], tylko luka nie jest stała, lecz mierzona
    /// w jednostkach zmienności — 20 punktów w spokojny wtorek i 20 punktów po
    /// danych z USA to dwie zupełnie różne decyzje. ATR liczy `atr_proxy`
    /// (zakres max−min w oknie), czyli estymator zakresowy, nie kwadraty
    /// zwrotów.
    Atr,
    /// SL o lukę `trail_atr_mult × ATR` od EKSTREMUM osiągniętego przez koszyk.
    ///
    /// Chandelier Exit (Chuck LeBeau): kotwicą jest szczyt (kupno) albo dołek
    /// (sprzedaż) od otwarcia koszyka, nie cena bieżąca. Różnica jest istotna
    /// dokładnie wtedy, gdy rynek przyspiesza: ATR wtedy rośnie, więc luka
    /// liczona OD CENY rozszerza się w chwili, w której chciałoby się
    /// zacisnąć — a luka liczona OD SZCZYTU nie może zejść poniżej raz
    /// osiągniętego poziomu inaczej niż przez wzrost samego ATR.
    Chandelier,
    /// bez trailingu
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailSrScope {
    /// tylko runner (ta sama definicja co w istniejącym passie trailingu:
    /// `trail_runners_by_depth` ? runnerzy wg głębokości : `p.is_runner`)
    Runner,
    Tp3Up,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailSrActivation {
    Entry,
    /// od zysku ≥ `trail_sr_min_gain` $ na uncji
    Gain,
    /// po dotknięciu TP1
    Tp1,
    /// po dotknięciu TP2 (środek plaskowyżu — wartość domyślna)
    Tp2,
    /// po dotknięciu TP3
    Tp3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaStateSrc {
    /// floating podzielony przez sumę R pierwotnych żywych koszyków —
    /// „ile stopów jestem pod wodą". Domyślne, bo nie zależy od tego, ile
    /// kapitału stoi na koncie.
    FloatR,
    /// floating jako procent equity — prostsze, ale przy kredycie bonusowym
    /// mierzy inną wielkość na koncie 300 i na koncie 300 + 300
    FloatPctEquity,
}

/// ZAPADKA STANU warstwy EA (`ea_state_ratchet`, niezmiennik N10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaRatchet {
    /// Koszyk zawiązany w stanie bardziej ostrożnym DOŻYWA w nim: powrót
    /// portfela do Neutral nie rozszerza trailingu i nie odblokowuje dokładek
    /// w koszyku, który już jest otwarty. Nowe koszyki dostają luźniejsze
    /// parametry. Zapadka kasuje się z zamknięciem koszyka.
    NieLuzujWKoszyku,
    /// Bez zapadki — stan portfela obowiązuje wszystkie koszyki natychmiast,
    /// w obie strony. Istnieje WYŁĄCZNIE po to, żeby dało się zmierzyć koszt
    /// zapadki; nie jest wariantem produkcyjnym.
    Swobodny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaStanDnia {
    /// Zachowanie dzisiejsze — dzień stratny nie zmienia ani jednej decyzji.
    Off,
    /// Po `ea_stan_dnia_prog_sl` STRATNYCH stopach w dobie:
    ///  * **twardo** — ani jednej dokładki (`fast_addon`, `rearm`, `re-entry`,
    ///    piramida). To jest niezmiennik, nie pokrętło,
    ///  * **miękko** — nowy koszyk dostaje `ea_stan_dnia_jednostki_mult`
    ///    jednostek (mnożnik przycięty do `(0; 1]`, więc nigdy nie podnosi).
    ///
    /// Pokrycie sygnałów zostaje NIETKNIĘTE: bot dalej gra każdy walidny
    /// setup, tylko mniejszym rozmiarem i bez dokładania do przegranych.
    TylkoInkaso,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseAllScope {
    Global,
    /// „close all" zamyka WYŁĄCZNIE koszyk-adresata (`target_basket`: reply_to
    /// → wskazówka cenowa → najnowszy żywy koszyk tego źródła), dokładnie tą
    /// samą drogą co pozostałe komunikaty zarządzające.
    Basket,
}

/// Jak liczyć offsety strefy wejścia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneOffsetMode {
    /// bez zmian — strefa dokładnie z sygnału
    None,
    /// offsety czysto cenowe (górna/dolna) — uwaga: dla SELL role są odwrócone
    Price,
    /// offsety wg JAKOŚCI wejścia: `deep` w stronę lepszych, `tol` poza krawędź
    Directional,
}

/// Co zrobić z poziomem siatki, na którym zlecenie OCZEKUJĄCE nie może leżeć.
///
/// Dotyczy dwóch sytuacji, które broker odrzuca tym samym kodem `10015`:
/// poziom po złej stronie rynku (limit kupna nad ceną) oraz poziom bliżej ceny
/// niż `stops_level`. Symulator od zawsze wchodził wtedy PO RYNKU, a żywy most
/// wysyłał zlecenie nie do przyjęcia i kończył na „rozstawiono 0 zleceń" —
/// czyli czempion po cichu omijał część sygnałów na koncie, wyglądając w
/// backteście na najlepszy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingCrossPolicy {
    /// wejście PO RYNKU — zachowanie symulatora i wartość domyślna
    Market,
    /// zamiana na zlecenie STOP po tej samej cenie (wejście dopiero z powrotem ceny)
    Stop,
    /// przesunięcie poziomu na najbliższy dopuszczalny (tuż za `stops_level`)
    Shift,
    /// pominięcie poziomu — jedyny wariant, który ŚWIADOMIE rezygnuje z wejścia
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketEntryMode {
    /// Wszystkie jednostki wszystkich poziomów w JEDNYM ticku po jednej cenie.
    /// Zachowanie sprzed rozdzielenia pól i wartość domyślna — preset, który
    /// nic o tym polu nie wie, dostaje dokładnie to, co mierzył dotąd.
    GridAtOnce,
    /// JEDNA pozycja rynkowa niosąca łączny rozmiar całego planu, przycięta
    /// limitem ryzyka koszyka liczonym od ceny wypełnienia. Sens: skoro
    /// wszystkie jednostki i tak wchodzą po jednej cenie, to jest jedna
    /// pozycja — rozbicie na pięć zleceń było wyłącznie księgowe, a przy
    /// okazji rozjeżdżało cele (`tp_schedule` rozdziela je po poziomach,
    /// których tu nie ma).
    Single,
    /// Jednostki UWALNIANE stopniowo: kolejny szczebel wchodzi dopiero, gdy
    /// cena przesunie się o `market_entry_step` na korzyść wejścia (dla kupna:
    /// w dół). To jest to, co opis `market_entry_step` obiecywał od zawsze,
    /// a czego kod nie robił — pole było czytane wyłącznie w `reentry_pass`.
    ///
    /// Szczeble idą od NAJPŁYTSZEGO do najgłębszego, bo wejście rynkowe
    /// startuje przy gorszej krawędzi strefy (albo za nią) i dopiero ruch
    /// ceny odsłania lepsze poziomy.
    Laddered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFreeRunnerStop {
    /// Stop na ŚREDNIEJ WAŻONEJ cenie wejścia koszyka i tam zostaje.
    /// Semantyka autora kanału i punkt odniesienia dla pozostałych trybów.
    Be,
    BeOwn,
    /// Stop na średniej TYLKO dopóki runner nie odjedzie; potem luźna
    /// zapadka o luzie `riskfree_runner_gap`. Dół nadal zamknięty, ale
    /// góra nie jest ścinana przez szum.
    TrailGap,
    /// Bez stopu do końca życia koszyka — ryzyko ogranicza wtedy wyłącznie
    /// `riskfree_runner_max_hold_h`.
    Off,
}

/// PODSTAWA WIELKOŚCI POZYCJI — od czego liczy się lot bazowy.
///
/// # Dlaczego to jest oś, a nie kosmetyka
///
/// `Balance` (dotychczasowe, domyślne) ma jedną własność, która przy
/// katastrofie marginesowej jest zabójcza: **saldo nie drga podczas
/// narastania straty pływającej**. Zmienia się dopiero przy ZAMKNIĘCIU
/// pozycji. Konto, które właśnie traci połowę wartości, ma więc przez cały
/// czas trwania zdarzenia ten sam lot bazowy, ten sam plan siatki i ten sam
/// cel relotu — czyli KAŻDY mechanizm liczony od salda (dynamiczny lot,
/// relot w dół, dławik) reaguje dopiero PO fakcie, gdy strata się zrealizuje.
///
/// `Equity` liczy podstawę od kapitału z uwzględnieniem pozycji otwartych.
/// Strata pływająca natychmiast obniża lot bazowy, więc plan siatki, relot
/// w dół i limity ryzyka zaczynają odlewarowywać rachunek **w trakcie**
/// zdarzenia, a nie po nim.
///
/// `MinOfBoth` bierze mniejszą z dwóch: rośnie ostrożnie jak saldo, ale
/// kurczy się natychmiast jak equity. To wariant asymetryczny — hamuje
/// szybko, przyspiesza wolno.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodstawaLota {
    Balance,
    /// kapitał z pozycjami otwartymi — reaguje na stratę pływającą od razu
    Equity,
    /// mniejsza z dwóch: hamuje jak equity, przyspiesza jak saldo
    MinOfBoth,
}

fn podstawa_lota_domyslna() -> PodstawaLota {
    PodstawaLota::Balance
}

/// Co zrobić z sygnałem idącym pod trend wyższego rzędu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendFilterMode {
    /// Nie wchodź wcale.
    Block,
    /// Wejdź MNIEJSZYM rozmiarem (`trend_filter_shrink`).
    ///
    /// Wariant równorzędny, nie zapasowy: twarda blokada przeciw dominującemu
    /// kierunkowi sygnałów może wyciąć większość handlu, przez co test mierzy
    /// brak handlu zamiast samego filtra.
    Shrink,
}

/// Czyje niezrealizowane limity liczą się przy ocenie „czy trzymać".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingScope {
    /// tylko limity TEGO koszyka — one uśredniają cenę tej pozycji
    SameBasket,
    /// dowolne nasze limity w tym kierunku — wsparcie dla ceny, choć bez uśredniania
    AnyBasket,
}

/// Które kierunki sygnału bot w ogóle rozpatruje.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SideFilter {
    Both,
    BuyOnly,
    SellOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DdGuardScope {
    /// Obsunięcie liczone od szczytu DNIA, blokada wygasa o północy serwera.
    Daily,
    /// Obsunięcie od szczytu wszech czasów, blokada do ręcznego wznowienia.
    Lifetime,
    /// Obsunięcie od szczytu wszech czasów, ale blokada wygasa o północy.
    LifetimePeakDailyReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TpSource {
    /// Wyłącznie cena z MT5. Komunikaty z kanału całkowicie ignorowane.
    PriceOnly,
    /// Wyłącznie komunikaty z kanału.
    SignalOnly,
    /// Oba niezależnie — liczy się to, co przyjdzie pierwsze.
    Either,
    /// Cena decyduje; komunikat przyspiesza etap tylko wtedy, gdy cena
    /// potwierdza go w granicach `tp_price_tolerance`.
    SignalConfirmedByPrice,
    /// Komunikat akceptowany, ale nie wcześniej niż `tp_signal_max_lead_s`
    /// sekund przed faktycznym dotknięciem ceny (kompromis: bierzemy lekko
    /// spóźnione i lekko wyprzedzające, odrzucamy ewidentnie błędne).
    PriceFirstSignalWindow,
}

/// Po jakiej CENIE oceniać reżim przy wpuszczaniu sygnału.
///
/// Filtr pytał dotąd zawsze o cenę rynkową z chwili przyjścia wiadomości.
/// Dla sygnału z limitem to złe pytanie: zlecenie wypełni się po CENIE
/// WEJŚCIA, często wiele godzin później. Sygnał „BUY LIMIT 4083-4085"
/// przychodzący przy cenie 4100 był odrzucany, mimo że wypełniłby się po
/// 4084 — czyli po cenie, która reżim przechodzi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeCena {
    Rynkowa,
    /// cena wejścia z sygnału (dla strefy: krawędź, po której wchodzimy)
    Wejscia,
    /// obie muszą przejść — najostrożniejszy wariant
    Obie,
}

/// Czym jest „próg" w filtrze reżimu.
///
/// Sama średnia jest miarą prymitywną: w instrumencie o zmienności złota
/// jeden gwałtowny ruch przeciąga ją na tyle, że próg przestaje opisywać
/// rynek. Mediana ignoruje szpilki, a środek kanału (min+max)/2 mówi, gdzie
/// cena stoi wobec ZAKRESU okna, a nie wobec jego środka ciężkości.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeMiara {
    Srednia,
    /// mediana okna — odporna na pojedyncze szpilki
    Mediana,
    /// środek kanału: (min + max) / 2
    Kanal,
    /// Średnia WYKŁADNICZA — ostatnie godziny ważą więcej.
    ///
    /// Średnia płaska z 72 punktów traktuje cenę sprzed trzech dni tak samo
    /// jak sprzed godziny. W instrumencie, który potrafi w dobę przejść kilkaset
    /// dolarów, to znaczy, że próg opisuje rynek, którego już nie ma.
    Wykladnicza,
    /// PERCENTYL okna — patrz `regime_percentyl`.
    ///
    /// Odpowiada na inne pytanie niż średnia: nie „po której stronie środka
    /// ciężkości", tylko „jak nisko w ZAKRESIE ostatnich dni". Przy kupnie
    /// przeciw trendowi interesuje nas dolna część zakresu, a ta nie musi
    /// pokrywać się z żadną średnią.
    Percentyl,
}

/// Co robić, gdy bramka zmienności uzna próg za nieaktualny.
///
/// Pierwsza wersja umiała tylko MILCZEĆ. To jest poprawne, ale marnotrawne:
/// kiedy zakres 72 godzin jest rozerwany, próg długiego okna faktycznie
/// opisuje rynek, którego już nie ma — ale okno krótkie jest wtedy DOKŁADNIE
/// tym, które jest aktualne. Filtr nie musi tracić zdania; może przesiąść się
/// na horyzont, który nadąża.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeGdyRozerwany {
    /// nie mam zdania — przepuść (zachowanie z pierwszej wersji)
    #[serde(alias = "Pass")]
    Milcz,
    /// licz próg z `regime_okno2_h` zamiast z `regime_ma_hours`
    KrotkieOkno,
    /// wejdź, ale MNIEJSZYM ROZMIAREM (tryb miękki `regime_soft_*`)
    ///
    /// Milczenie przepuszcza sygnał w PEŁNYM rozmiarze — a to znaczy, że
    /// najagresywniej gramy dokładnie wtedy, gdy przyznajemy się do niewiedzy.
    /// Tryb miękki mówi zamiast tego „nie wiem, więc mniejszą stawką": na
    /// koncie live (600 $, kredyt 300 odjęty) podnosi margines minimalny
    /// ze 101 % na 118 %, czyli zdejmuje ryzyko wezwania do uzupełnienia.
    #[serde(alias = "Soft")]
    Miekko,
}

/// Filtr reżimu rynku.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeFilter {
    Off,
    /// handluj tylko zgodnie z nachyleniem średniej z N godzin
    TrendMa,
    /// handluj tylko PRZECIW nachyleniu (fade)
    CounterMa,
}

/// ROZMIAR STEROWANY ZMIENNOŚCIĄ — mnożnik lota, nie bramka.
///
/// Różnica wobec `regime_filter` jest zasadnicza i to ona jest powodem, dla
/// którego to pole w ogóle powstało. Filtr reżimu ma dwa stany (handluj /
/// nie handluj), więc jego geometria włączeń sama niesie wynik — placebo
/// z zachowaną geometrią odtworzyło +6,9 pp z +10,3 pp przewagi CounterMa.
/// Mnożnik jest CIĄGŁY: nie ma dnia, w którym „nie handlujemy", jest tylko
/// dzień, w którym gramy mniejszym lotem. Nie da się go podrobić losowaniem
/// terminów, bo nie ma terminów.
///
/// # Kontrakt zera
///
/// [`VolSizeMode::Off`] znaczy mnożnik dokładnie `1.0`, a `lot_size()` przy
/// `Off` w ogóle nie wykonuje mnożenia. To jest STRUKTURALNY parytet: nie
/// „mnożymy przez jedynkę i ufamy, że f64 tego nie ruszy", tylko nie wchodzimy
/// w tę gałąź.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolSizeMode {
    /// wyłączone — mnożnik 1,0, ścieżka parytetu
    Off,
    /// waga = `vol_size_target / zmienność_odsezonowana` (Harvey/Hoyle/Rattray).
    /// Docelowy zasięg w USD: ile ma się ruszać rynek, żeby lot był bazowy.
    Target,
    Percentile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankRounding {
    /// klasyczne zaokrąglenie — pułapka „15 % z 3 = 0"
    Nearest,
    /// w górę, minimum jedna pozycja
    Up,
    /// w dół — transza może wyjść pusta
    Down,
}

/// Od której strony koszyka pobierać transzę do zamknięcia na celu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankFrom {
    /// zamykaj NAJGORSZE wejścia, najlepsze zostają runnerami (styl grupy)
    Worst,
    /// zamykaj NAJLEPSZE — inkasuje pewny zysk, zostawia nadzieję
    Best,
}

/// Co dostaje ostatnia pozycja koszyka po wyczerpaniu harmonogramu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastRunner {
    /// najdalszy cel drabinki (albo „TP OPEN", gdy cele wolno przedłużać)
    Runner,
    /// kolejny nietrafiony szczebel drabinki — szybszy bank, krótszy ogon
    NextTp,
    /// bez celu w ogóle: pozycja jedzie wyłącznie na trailingu
    NoTp,
}

/// Schodkowy stop-loss zależny od RANGI pozycji w koszyku.
///
/// Ranga 0 = najlepsze wejście (BUY: najniższa cena). Po n-tym celu n
/// najlepszych pozycji dostaje kolejne szczeble łańcucha, więc pozycja
/// o najlepszej cenie ma najmocniej podciągnięty SL. To odwzorowanie
/// „SMART SL TRAILING" z poprzedniego bota — jedyna rzecz, która różnicowała
/// ochronę wewnątrz jednego koszyka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmartSlMode {
    Off,
    /// łańcuch: SL sygnału → TP1 → TP2 → …
    Ladder,
    /// łańcuch: SL sygnału → breakeven (i koniec)
    BreakevenOnly,
    /// łańcuch: SL sygnału → breakeven → TP1 → TP2 → …
    LadderWithBe,
}

// ============================================================
//  KONFIGURACJA
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---------- wielkość pozycji ----------
    pub lot_mode_percent: bool,
    pub lot_fixed: f64,
    pub lot_percent: f64,
    /// +0.01 lota na każde X $ balansu (compounding). 0 = stały lot.
    pub lot_scale_step: f64,
    pub lot_max: f64,
    pub lot_min: f64,
    /// Globalny kontrakt końcowego wolumenu otwarcia. OFF zachowuje legacy.
    /// ON: po wszystkich mnożnikach floor do kroku brokera, min/cap brokera
    /// i strategii; żądanie nigdy nie jest podnoszone. Nieznane/nieważne
    /// parametry i brak legalnego lota oznaczają jawną odmowę nowego zlecenia.
    /// Nie zmienia wolumenu zamknięć ani partiali, nie podnosi uprawnień bota.
    pub order_volume_contract_v2: bool,

    // ---------- strefa wejścia ----------
    pub zone_offset_mode: ZoneOffsetMode,
    pub entry_hi_offset: f64,
    pub entry_lo_offset: f64,
    pub entry_deep_offset: f64,
    #[serde(default)]
    pub entry_deep_frac_to_sl: f64,
    pub entry_tol_offset: f64,
    /// ignoruj sygnały rynkowe, bierz tylko LIMIT
    pub only_limit_signals: bool,
    pub auto_limit: bool,
    /// anuluj sygnał starszy niż X minut (0 = OFF)
    pub ignore_old_after_min: f64,
    pub skip_if_sl_breached: bool,
    pub max_chase_beyond_zone: f64,

    // ---------- siatka ----------
    pub entry_units: u32,
    /// osobna liczba jednostek dla koszyków LIMIT (0 = jak wyżej)
    pub entry_units_limit: u32,
    /// gęstość siatki: krok = 1/ppm (0 = jeden poziom na krawędzi)
    pub ppm: f64,
    pub ppm_enabled: bool,
    /// budżet ryzyka na poziom: jednostki = clamp(budżet/dystans do SL, 1, units)
    pub entry_risk_budget: f64,
    /// sizing z geometrii: jednostki = clamp(budżet/|TP1 − poziom|, 1, units)
    pub entry_tp1_budget: f64,
    /// Wagi wolumenu wg GŁĘBOKOŚCI wejścia w strefie — „1,2,4".
    ///
    /// Czytane **od najpłytszego wejścia do najgłębszego**. Puste = wszystkie
    /// poziomy równe (dotychczasowe zachowanie).
    ///
    /// Dlaczego to jest potrzebne: w strefie o medianie 5 $ szerokości wejście
    /// przy płytkiej krawędzi ma SL 6 $ i TP1 3 $ (R:R 0,50), a przy głębokiej
    /// SL 1 $ i TP1 8 $ (R:R 8,0). Szesnastokrotna różnica jakości. Równe
    /// wolumeny na całej drabince oznaczają, że najgorsze wejście waży tyle
    /// samo co najlepsze — i to ono decyduje o wyniku. Rachunek z danych:
    /// kwiecień +1293 $, maj −198 $, czerwiec −179 $.
    ///
    /// Wagi **redystrybuują** wolumen, nie powiększają go: mnożniki są
    /// normowane do średniej 1, więc sumaryczna wielkość koszyka zostaje taka
    /// sama jak bez wag. Za wielkość bezwzględną odpowiada `risk_per_basket_pct`.
    pub entry_weights: String,
    pub entry_uklad: String,
    pub entry_uklad_kotwica: String,
    pub entry_krzywa_kotwica: String,
    pub tp_drabinka_kotwica: String,
    pub entry_depth_curve: f64,

    #[serde(default)]
    pub entry_allowance_usd: f64,
    /// Ile JEDNOSTEK zlecenia stawiać na warstwie allowance (0 = żadnej).
    ///
    /// Liczone tak samo jak `base_units` zwykłego szczebla: tyle zleceń
    /// o wolumenie bazowym ląduje na jednej cenie `zone_hi + entry_allowance_usd`
    /// (BUY) / `zone_lo − entry_allowance_usd` (SELL). Warstwa NIE dzieli się
    /// budżetem `entry_units` — jest dołożona PONAD siatkę wewnątrz strefy,
    /// więc `entry_units = 5` + `entry_allowance_units = 2` to plan siedmiu
    /// jednostek. Limit ryzyka koszyka (`risk_per_basket_pct`) i sufit
    /// portfelowy widzą ją jak każdą inną warstwę, bo doklejamy ją PRZED
    /// `cap_basket_risk`.
    ///
    /// Wycena z DIFF-u STORM: ~+140 $ przy 2 jednostkach, ~+350 $ przy pełnych
    /// 5 — na oknie, na którym te strefy nie widziały stopa.
    ///
    /// Kontrakt zera: `0` (domyślnie) = warstwy nie ma.
    #[serde(default)]
    pub entry_allowance_units: u32,

    pub risk_per_basket_pct: f64,
    /// dodatkowe jednostki na krawędzi strefy z własnym wczesnym celem
    pub toucher_units: u32,
    pub toucher_tp_index: usize,
    /// Czy `toucher_tp_index` liczy cele od JEDYNKI (domyślnie `false` = od zera).
    ///
    /// Rozbieżność wewnątrz naszego własnego kodu: `toucher_bands` parsuje
    /// trzeci człon pasma jako numer celu OD JEDYNKI (`saturating_sub(1)`),
    /// a `toucher_tp_index` bez pasm szedł surowo jako indeks od zera.
    /// Preset przepisany z `bot.py` (`entry_touch_tp: 2`, gdzie `bot.py`
    /// liczy `tps[max(1,n)-1]`, linia 6777) celował więc w TP3 zamiast w TP2.
    /// `true` = numeracja `bot.py`.
    pub toucher_tp_one_based: bool,
    /// pasma głębokości „offset_pips:units:tp_index" po przecinku
    pub toucher_bands: String,
    pub pending_lifetime: PendingLifetime,
    pub pending_drop_on_target: bool,
    pub pending_drop_arm: bool,
    /// kasuj pendingi koszyka starsze niż X h (0 = OFF)
    pub pending_ttl_h: f64,
    /// TTL liczony od powstania KOSZYKA, nie od wystawienia zlecenia.
    ///
    /// Poprzedni bot mierzył wiek koszyka i to jest właściwa semantyka:
    /// „stare setupy wypełniają się dopiero w krachu". Mierzenie wieku
    /// pojedynczego zlecenia rozjeżdża się z nią po każdym re-armie siatki.
    pub pending_ttl_from_basket: bool,
    /// Krok siatki dotyczy zleceń oczekujących (PPM „FOR LIMITS").
    pub ppm_for_limits: bool,
    pub grid_anchor_absolute: bool,
    #[serde(default = "default_units_per_level")]
    pub units_per_level: bool,
    #[serde(default = "default_units_per_level_zone")]
    pub units_per_level_zone: bool,
    /// Co zrobić z poziomem, na którym limit nie może leżeć (patrz
    /// `PendingCrossPolicy`). Domyślnie `Market` — dokładnie to, co robi
    /// symulator, więc backtest i konto opisują ten sam handel.
    pub pending_cross_policy: PendingCrossPolicy,
    /// Krok dotyczy wejść RYNKOWYCH — czyli tego, o ile cena musi się
    /// przesunąć na naszą korzyść, zanim dołożymy kolejną pozycję.
    pub ppm_for_market: bool,
    pub market_entry_step: f64,
    /// Jak rozłożyć wejście idące PO RYNKU — patrz [`MarketEntryMode`].
    ///
    /// Domyślnie `GridAtOnce`, czyli zachowanie sprzed rozdzielenia tej
    /// decyzji od `auto_limit`. Pole nie jest czytane na ścieżce limitowej.
    pub market_entry_mode: MarketEntryMode,

    pub vol_window_min: f64,
    /// Zakres H−L, od którego rynek uznajemy za sztormowy.
    pub vol_range_usd: f64,
    /// Mnożnik liczby jednostek w sztormie (< 1 = tnij rozmiar).
    pub vol_units_mult: f64,
    /// Przeliczaj rozmiar NIEZAFILLOWANYCH limitów przy zmianie reżimu.
    ///
    /// Bez tego koszyk rozstawiony w sztormie zostaje mały nawet wtedy, gdy
    /// realizuje się godzinę później na spokojnym rynku — i odwrotnie. To był
    /// udokumentowany rozjazd między silnikiem (liczy przy fillu) a botem
    /// (zamrażał przy rozstawianiu).
    pub pending_resize_on_vol: bool,
    /// Co ile sekund przeliczać (0 = co tick, kosztowne).
    pub pending_resize_s: f64,
    /// PRZELICZANIE WOLUMENU LEŻĄCYCH LIMITÓW PO WZROŚCIE SALDA.
    ///
    /// Lot bazowy liczy się z salda W CHWILI SKŁADANIA zlecenia i już nigdy
    /// się nie zmienia. Przy compoundingu to znaczy, że siatka rozstawiona
    /// przy 200 $ wypełnia się lotem 0,01 także wtedy, gdy konto urosło
    /// w międzyczasie do 500 $ i normalnie handluje 0,03 — a najgłębsze
    /// szczeble czekają na wypełnienie najdłużej, czyli rozjazd dotyczy
    /// właśnie tych wejść, które dają najwięcej.
    ///
    /// MT5 nie pozwala zmienić wolumenu zlecenia oczekującego (`modify_pending`
    /// nie ma takiego parametru), więc jedyna droga to ANULUJ I ZŁÓŻ OD NOWA.
    /// To kosztuje: zlecenie traci miejsce w kolejce i może zostać odrzucone
    /// po ponownym złożeniu, a między anulowaniem a złożeniem jest okno,
    /// w którym szczebla NIE MA na rynku.
    #[serde(default)]
    pub pending_relot_on_balance: bool,
    /// Opt-in checked aggregate relot. Complete empty plan reduces to zero;
    /// invalid/unknown plan never authorizes cancellation. Overrides the legacy
    /// pending_relot_wg_planu choice. Uncertain replacement persists RequiresReview.
    #[serde(default)]
    pub pending_relot_reconcile_target: bool,
    /// SPOSÓB podniesienia wolumenu. `false` = anuluj i złóż od nowa.
    /// `true` = DOŁÓŻ osobne zlecenie na tę samą cenę, na samą różnicę.
    ///
    /// Dokładka jest bezpieczniejsza: pierwotny szczebel ani na chwilę nie
    /// znika z rynku, nie traci miejsca w kolejce, a odmowa brokera kosztuje
    /// tylko brakującą różnicę zamiast całego wejścia. Cena: na szczeblu leżą
    /// dwa zlecenia zamiast jednego (silnik traktuje je jak jedno).
    #[serde(default)]
    pub pending_relot_topup: bool,
    /// KIERUNEK W GÓRĘ — wolno DOKŁADAĆ wolumen, gdy szczebel jest mniejszy
    /// niż cel. To strona ZYSKU: odzyskuje compounding, którego zamrożony lot
    /// nie dowoził. Wyłączenie daje wariant „tylko redukcja" (HYPER-X1A).
    ///
    /// Działa wyłącznie pod `pending_relot_on_balance`, więc preset, który
    /// o tym polu nie wie, dostaje liczby co do centa te same.
    #[serde(default = "prawda")]
    pub pending_relot_up: bool,
    /// KIERUNEK W DÓŁ — wolno ZMNIEJSZAĆ szczebel, gdy jest większy niż cel.
    /// To strona RYZYKA: zlecenie złożone przy dużym saldzie leżące, gdy konto
    /// spadło, otwiera pozycję wielokrotnie za dużą względem kapitału.
    #[serde(default = "prawda")]
    pub pending_relot_down: bool,
    /// PRÓG KAPITAŁU dla kierunku w górę. Dokładanie wolumenu włącza się
    /// dopiero, gdy saldo sięgnie tej kwoty; poniżej działa sama redukcja.
    ///
    /// Odpowiada na pytanie użytkownika: „konto zaczyna od wersji łagodnej
    /// przy 300 $, a po urośnięciu przełącza się na pełną — czy to ma sens?".
    /// `0` = bez progu, czyli zachowanie sprzed dołożenia pola.
    #[serde(default)]
    pub pending_relot_up_od_salda: f64,
    #[serde(default = "prawda")]
    pub pending_relot_wg_planu: bool,

    #[serde(default)]
    pub expo_cap_pct: f64,
    /// Czy po skasowaniu WSZYSTKICH leżących szczebli wolno jeszcze DOMYKAĆ
    /// POZYCJE, żeby zejść pod próg.
    ///
    /// Rozdzielone świadomie: kasowanie limitu nie kosztuje nic poza utratą
    /// wejścia, którego jeszcze nie ma, a domykanie pozycji REALIZUJE stratę
    /// i wyłącza koszyk z odbicia. Jeśli sam wariant (a) wystarcza, tego pola
    /// nie wolno włączać — prostsze wygrywa.
    #[serde(default)]
    pub expo_cap_close: bool,
    /// Kadencja sprawdzania w sekundach. `0` = co tick.
    ///
    /// Ma znaczenie wyłącznie przy włączonym `expo_cap_pct` albo
    /// `expo_cap_ml_pct`; przy obu wyłączonych funkcja nie dochodzi tutaj.
    #[serde(default)]
    pub expo_cap_s: f64,
    #[serde(default)]
    pub expo_cap_ml_pct: f64,

    #[serde(default)]
    pub ml_licz_wiszace: bool,
    /// Minimalny poziom marginesu, przy którym wolno OTWORZYĆ NOWY KOSZYK.
    ///
    /// Uzupełnia `margin_call_level_pct`, które stoi w tym samym miejscu, ale
    /// domyślnie na 50 % — czyli tam, gdzie broker już dzwoni. Ta oś pozwala
    /// zatrzymać się wcześniej.
    #[serde(default)]
    pub ml_min_wejscie: f64,
    /// Minimalny poziom marginesu na KAŻDĄ KOLEJNĄ WARSTWĘ siatki.
    ///
    /// Dziś siatka planowana jest raz, a potem rozstawiana bez ani jednego
    /// spojrzenia na rachunek. Ta oś przerywa rozstawianie w połowie, gdy
    /// dokładanie przestaje być bezpieczne — koszyk zostaje z tym, co już
    /// weszło, zamiast dociągnąć plan do końca wbrew stanowi konta.
    #[serde(default)]
    pub ml_min_warstwa: f64,
    /// Minimalny poziom marginesu na RE-ENTRY po trafionym celu.
    #[serde(default)]
    pub ml_min_reentry: f64,
    /// Minimalny poziom marginesu na PRZEZBROJENIE siatki (`rearm`).
    #[serde(default)]
    pub ml_min_rearm: f64,
    /// Minimalny poziom marginesu na PIRAMIDĘ.
    ///
    /// Dziś piramida nie ma ANI JEDNEJ bramki ekspozycji — ani sztuk, ani
    /// lotów, ani dolarów, ani wywołania `entry_gate`. Jest nieszkodliwa
    /// wyłącznie dlatego, że `pyramid_after_stage = 0` we wszystkich wydanych
    /// presetach wyłącza całą gałąź. Ta oś jest warunkiem, pod którym wolno
    /// ją komukolwiek włączyć.
    #[serde(default)]
    pub ml_min_piramida: f64,
    /// Minimalny poziom marginesu na DOKŁADKĘ PO SZYBKIM RUCHU (`fast_addon`).
    ///
    /// Ta ścieżka OMIJA `entry_gate` — sprawdza wyłącznie licznik pozycji,
    /// mimo komentarza obok niej twierdzącego, że „bramka ekspozycji
    /// obowiązuje TAK SAMO jak przy zwykłym wejściu".
    #[serde(default)]
    pub ml_min_fast_addon: f64,
    /// Minimalny poziom marginesu na RELOT W GÓRĘ (powiększanie wiszących
    /// zleceń po wzroście salda).
    #[serde(default)]
    pub ml_min_relot_up: f64,
    /// Minimalny poziom marginesu na SZCZEBEL DRABINY RYNKOWEJ.
    #[serde(default)]
    pub ml_min_drabina: f64,
    #[serde(default)]
    pub konto_dzwignia: f64,
    #[serde(default)]
    pub wiek_od_wypelnienia: bool,
    #[serde(default)]
    pub pending_drop_grace_min: f64,
    pub pending_drop_grace_max_dist: f64,
    pub pending_drop_keep_n: u32,
    #[serde(default = "podstawa_lota_domyslna")]
    pub lot_base: PodstawaLota,

    // ---------- stop loss ----------
    /// minimalna odległość SL od środka strefy (0 = SL z sygnału bez zmian)
    pub sl_min_dist: f64,
    /// maksymalna odległość SL (0 = bez limitu); szerszy SL → nie otwieraj
    pub sl_max_dist: f64,
    /// nie otwieraj wejścia oddalonego od SL o więcej niż X (0 = OFF)
    pub entry_sl_dist_limit: f64,
    /// SL trzymany u bota, broker widzi tylko siatkę ratunkową
    pub virtual_sl: bool,
    pub virtual_sl_only_when_rejected: bool,
    pub virtual_sl_all: bool,
    /// co ile sekund sprawdzać wirtualny SL (0 = co tick)
    pub vsl_eval_s: f64,
    /// o ile odsunąć SL brokera przy wirtualnym SL
    pub vsl_broker_offset: f64,

    // ---------- cele ----------
    pub tp_schedule: TpSchedule,
    pub scale_out_pct: f64,
    pub official_pct: [f64; 4],
    pub official_counts: String,
    /// po TP3 zamykaj % na każdym kolejnym celu
    pub official_spp: bool,
    /// każda pozycja dostaje własny TP wg harmonogramu (MT5 zamyka sam)
    pub assign_tp_per_position: bool,
    /// krok generowania kolejnych celów po wyczerpaniu drabinki
    pub tp_open_offset: f64,
    /// po wyczerpaniu drabinki cel runnera ZOSTAJE (nie ucieka przed ceną)
    pub tp_freeze_after_ladder: bool,
    pub tp_open_extra: bool,
    /// Skąd bot czerpie wiedzę o trafieniu celu i co ma pierwszeństwo.
    pub tp_source: TpSource,
    /// Tolerancja ceny przy potwierdzaniu komunikatu (różnice spreadów
    /// między brokerem sygnalisty a naszym).
    pub tp_price_tolerance: f64,
    /// Autonomiczne wyprzedzenie celu wyłącznie na podstawie Bid/Ask MT5.
    ///
    /// `0` = kontrakt legacy: cel cenowy jest wykonany dopiero przy pełnym
    /// dotknięciu. Wartość `> 0` wykonuje NASTĘPNY etap `offset` USD przed
    /// jego poziomem (BUY: `bid >= tp - offset`, SELL: `ask <= tp + offset`).
    /// Nie czyta Telegrama i działa nawet przy `tp_source = SignalOnly`.
    /// Dotyczy tylko koszyka z otwartą pozycją: nie skraca życia samej
    /// siatki oczekującej. Przechodzi tą samą idempotentną ścieżką
    /// `handle_tp_hit` co zwykłe dotknięcie ceny.
    pub tp_price_front_run_usd: f64,
    /// Ile sekund PRZED faktycznym dotknięciem ceny wolno przyjąć komunikat
    /// (tryb `PriceFirstSignalWindow`). 0 = wymagaj potwierdzenia ceną.
    pub tp_signal_max_lead_s: f64,
    /// Ile sekund PO fakcie komunikat jest jeszcze uznawany za aktualny.
    /// 0 = bez limitu (spóźniony komunikat zawsze akceptowany).
    pub tp_signal_max_lag_s: f64,
    /// Czy REALIZACJA zlecenia u brokera (pozycja zamknięta na swoim TP)
    /// ma przesuwać etap koszyka. To najtwardszy dowód trafienia celu.
    pub tp_stage_from_broker_fill: bool,
    /// „TP3 HIT" przy etapie 0 domyka też pominięte etapy
    pub tp_hit_fill_stages: bool,
    /// Zaokrąglanie transzy pozycji zamykanych na celu.
    pub bank_rounding: BankRounding,
    /// Od której strony koszyka brać transzę.
    pub bank_from: BankFrom,
    /// Czy wolno zamknąć OSTATNIĄ pozycję koszyka wg harmonogramu.
    ///
    /// `false` (domyślnie) zawsze zostawia jednego runnera — tak prowadzi
    /// koszyk sygnalista („if you have any runners just HOLD"). `true` jest
    /// dla konfiguracji, które wolą domknąć koszyk niż trzymać ogon.
    pub bank_close_last: bool,
    /// Co dostaje ostatnia pozycja, gdy harmonogram się skończył.
    pub last_runner: LastRunner,
    /// Zamykaj CZĘŚĆ WOLUMENU pozycji zamiast całych pozycji.
    ///
    /// 0.01 lota jest niepodzielny, a broker podnosi każde mniejsze zlecenie
    /// do 0.01 — przy małym locie „15 %" oznaczałoby w rzeczywistości 100 %
    /// pozycji. Dlatego tryb włącza się dopiero, gdy KAŻDA pozycja koszyka ma
    /// wolumen ≥ `partial_min_lot`; poniżej progu silnik sam wraca do
    /// zamykania całych pozycji i ten sam preset działa na koncie 200 $
    /// i 10 000 $.
    pub partial_close: bool,
    pub partial_min_lot: f64,

    pub partial_pct_od_pierwotnego: bool,

    pub cele_na_ostatnim: bool,

    /// Przy `cele_na_ostatnim` także późniejszy retarget zachowuje najdalszy
    /// cel (łącznie z jawnym TP OPEN offset). Nie przywraca usuniętego TP.
    /// OFF zachowuje historyczny retarget na następny etap, który potrafi
    /// skrócić cel mimo `cele_na_ostatnim`. ON nie dopisuje kolejnego offsetu
    /// po końcu drabinki: celem pozostaje cel wyznaczony przez target_for_ex.
    pub retarget_respects_final_target: bool,

    pub sl_polowa_od_konca: usize,

    /// Jaki UŁAMEK drogi wejście → cena bierze reguła `sl_polowa_od_konca`.
    ///
    /// `0,5` to litera opisu („halfway"). Pole istnieje, bo połowa jest
    /// wyborem tradera, a nie stałą przyrodniczą — `0,33` zostawia runnerowi
    /// więcej powietrza, `0,75` domyka agresywniej. `0` czyta się jak `0,5`,
    /// żeby preset z włączoną regułą i niewypełnionym ułamkiem nie ustawiał
    /// stopu dokładnie na wejściu (to już robi `be_at_tp1`).
    ///
    /// Stop przesuwa się WYŁĄCZNIE w stronę zysku i tylko wtedy, gdy wychodzi
    /// lepiej niż obecny — reguła jest zapadką. Cofanie stopu nie jest tym,
    /// co ktokolwiek ma na myśli, mówiąc „przesuwam SL na połowę".
    pub sl_polowa_ulamek: f64,
    /// Maksymalny wiek koszyka (godziny), przy którym komunikat
    /// „SECURING PARTIAL PROFITS" wolno jeszcze przezbroić nową drabinką celów.
    ///
    /// Stare koszyki z żywymi runnerami dostawały nowe cele i etap zerowany —
    /// patologia ciągnąca się przez wiele dni. 0 = bez ograniczenia.
    pub spp_max_age_h: f64,
    /// „SECURING PARTIAL PROFITS" nie zmienia celów już otwartych pozycji
    /// (bierzemy z komunikatu tylko SL i sam fakt trafienia celu).
    pub spp_keep_tp: bool,
    pub spp_sl_mode: SppSlMode,
    /// Bufor (w dolarach ceny) odsuwający poziom sygnalisty OD ceny, z powrotem
    /// w stronę wejścia. 0 = dokładnie tam, gdzie napisał autor.
    ///
    /// Istnieje, bo poziom autora jest liczony dla JEGO wejścia (jedno wejście
    /// po rynku), a nasz koszyk ma pięć poziomów rozłożonych na 5 $ strefy —
    /// ten sam „break even" bywa dla naszych głębokich wejść stopem
    /// postawionym w środku szumu. Bufor zamienia binarny przełącznik w oś.
    pub spp_sl_pad: f64,

    // ---------- reakcje na komunikaty ----------
    pub risk_free_mode: RiskFreeMode,
    pub risk_free_runners: u32,
    pub risk_free_runner_target: RiskFreeRunnerTarget,
    pub risk_free_trail: bool,
    #[serde(default)]
    pub risk_free_be_min_profit: f64,
    pub out_at_entry_mode: OutAtEntryMode,
    /// Co zrobić na „OUT AT ENTRY" z pozycją POD WODĄ — patrz [`OaePodWoda`].
    /// Czytane WYŁĄCZNIE przy `out_at_entry_mode = MoveSlToBe`, bo tylko tam
    /// istnieje gałąź, która dziś kończy się niczym.
    #[serde(default = "oae_pod_woda_domyslna")]
    pub oae_pod_woda: OaePodWoda,
    pub oae_band_pts: f64,
    pub sl_hit_mode: SlHitMode,
    /// tolerancja weryfikacji fałszywego „SL HIT"
    pub sl_hit_verify_tol: f64,
    pub honor_cancel: bool,
    pub honor_close_all: bool,
    #[serde(default = "default_close_all_scope")]
    pub close_all_scope: CloseAllScope,
    #[serde(default)]
    pub partials_wykonuj: bool,
    #[serde(default)]
    pub partials_pct: f64,
    #[serde(default)]
    pub parser_luz_interpunkcyjny: bool,
    /// Nie wykonuj statystyk z podsumowań dnia/tygodnia jako poleceń TP.
    ///
    /// Zbiorcze podsumowanie może zawierać linie `TP1 Hit: N`, `TP2 Hit: N`
    /// i `TP3 Hit: N`. Bez osłony parser mógłby potraktować taką linię jak
    /// pojedyncze wykonanie celu i inkasować najnowszy żywy koszyk. Oś nie
    /// filtruje wejść: pełny blok sygnału ma w parserze pierwszeństwo.
    /// `false` zachowuje poprzednie działanie 1:1.
    #[serde(default)]
    pub recap_guard: bool,
    /// Traktuj `AT TPn` jako telemetry bliskosci, nigdy jako wykonawcze
    /// `TpHit`.
    ///
    /// Format może opublikować `AT TP1`, a dopiero późniejsza wersja lub
    /// wiadomość potwierdza `TP1 HIT`. `AT` oznacza bliskość celu;
    /// bankowanie w tej chwili teleportowalo etap przed faktycznym Bid/Ask.
    /// Filtr usuwa WYLACZNIE `Signal::TpHit` pochodzacy z `AT TP`; RF, SPP,
    /// CANCEL, BE/SL i przyszle cele z tej samej NEW/EDIT nadal ida normalna
    /// sciezka. Numerowane `TPn HIT` i konkretne `price HIT` sa zachowane jako
    /// informacja, a ich wykonanie kontroluje osobno `tp_source` (dla GOD-X5:
    /// `SignalConfirmedByPrice`). `false` = semantyka legacy 1:1.
    #[serde(default)]
    pub profit_update_telemetry_only: bool,
    /// Reaguj na „BUY NOW" / „SELL NOW" — otwarcie po rynku bez strefy.
    ///
    /// Domyślnie wyłączone: taki komunikat nie niesie ani SL, ani celów,
    /// więc pozycja powstałaby bez planu wyjścia.
    pub honor_market_open: bool,
    /// Tolerancja dopasowania komunikatu do koszyka po podanym poziomie.
    ///
    /// Komunikat „(4002.1 TO 4006)" wskazuje, o który sygnał chodzi. Bez tego
    /// każdy komunikat trafia do NAJNOWSZEGO koszyka źródła — a przy dwóch
    /// żywych koszykach to jest losowanie.
    pub basket_hint_tolerance: f64,
    /// Nie wykonuj ponownie akcji, które już wynikły z tej samej wiadomości.
    ///
    /// Sygnalista edytuje komunikat („TP1 HIT" → „TP1 HIT · SECURING PARTIAL
    /// PROFITS"). Bez pamięci wykonanych akcji edycja powtarza inkaso na TP1.
    pub dedup_edited_signals: bool,
    #[serde(default = "default_dedup_pelny_status")]
    pub dedup_pelny_status: bool,
    /// Edycja z wejściem wykonuje TAKŻE resztę akcji z tej samej wiadomości
    /// (Pakiet A3).
    ///
    /// Dziś edycja trafiająca w koszyk (`apply_entry_edit`) kończy obsługę
    /// wiadomości — doklejone w tej samej edycji „TP1 HIT" / „SECURING
    /// PARTIAL PROFITS" przepada bez śladu. Przy `true` po przezbrojeniu
    /// koszyka wiadomość idzie dalej normalną ścieżką (dedup + dispatch
    /// pozostałych akcji), a `entry` zapisuje się jako wykonana akcja tej
    /// wiadomości. `false` (domyślnie) = zachowanie dzisiejsze co do bitu.
    #[serde(default = "default_edycja_wykonuje_reszte_akcji")]
    pub edycja_wykonuje_reszte_akcji: bool,
    /// Klucz dedupu akcji NIESIE WARTOŚĆ (Pakiet A4).
    ///
    /// `action_key()` celowo gubi wartości, więc edycja zmieniająca POZIOM
    /// („MOVE SL TO 4120" → „MOVE SL TO 4110") ginie jako duplikat — klucz
    /// `setsl` już jest w pamięci. Przy `true` dedup liczy się kluczem
    /// `action_key_v2()` (`setsl@4110`, `corr2@4162`, `rf@4536`,
    /// `spp@…/…|sl:…|be:…`), więc ta sama akcja z INNĄ wartością przechodzi.
    /// `false` (domyślnie) = klucz v1, zachowanie dzisiejsze.
    #[serde(default = "default_dedup_klucz_z_wartoscia")]
    pub dedup_klucz_z_wartoscia: bool,
    #[serde(default = "default_edycja_sieroty_nie_otwiera")]
    pub edycja_sieroty_nie_otwiera: bool,
    #[serde(default = "default_entry_idempotencja")]
    pub entry_idempotencja: bool,
    /// Versioned source-aware entry edits. OFF preserves legacy mutation/order
    /// sequence. ON makes cosmetic edits no-op and quarantines uncertain
    /// geometry replacement per basket; it is not a durable broker checkpoint.
    #[serde(default)]
    pub entry_edit_geometry_v2: bool,
    /// Exact tick S/R warm-up contract (strategy scope). OFF preserves legacy
    /// warm-up. ON requires its own live/backtest warm-up readiness proof.
    #[serde(default)]
    pub sr_warmup_exact_ticks: bool,
    /// Trwaly dedup komunikatow ZARZADZAJACYCH przez restart (Pakiet A7).
    ///
    /// `done_actions` silnika bylo dotad wylacznie pamiecia RAM. Po
    /// `adopt_baskets` Telegram mogl dostarczyc ponownie te sama wiadomosc
    /// (albo jej kolejna edycje), a nienumerowane `+PIPS HIT` awansowalo wtedy
    /// o nastepny TP; ponowne SPP/BE moglo drugi raz zredukowac runnera.
    /// Przy `true` wykonane klucze akcji sa zapisane w migawce zywego koszyka
    /// i odtwarzane razem z nim. Ochrona obejmuje rowniez re-delivery jako
    /// `NEW`, nie tylko explicit `EDIT`. `false` (domyslnie) = stare
    /// zachowanie i pusty/brakujacy klucz w `koszyki.json`.
    #[serde(default)]
    pub dedup_management_po_restarcie: bool,
    /// Opcjonalny strażnik odróżniający warunkową zapowiedź RISK FREE od
    /// jednoznacznego polecenia. Polecenie z poziomem liczbowym pozostaje
    /// wykonalne; `false` zachowuje starszą semantykę parsera.
    #[serde(default = "default_rf_wymaga_wykonania")]
    pub rf_wymaga_wykonania: bool,
    #[serde(default = "default_market_entry_units")]
    pub market_entry_units: u32,
    /// Hybrydowe wykonanie sygnału BEZ `LIMITS`: N jednostek z PIERWSZEGO
    /// (najpłytszego) wejścia otwórz natychmiast po rynku, a pozostałą
    /// drabinkę pozostaw jako limity. 0 = OFF / zachowanie historyczne.
    ///
    /// To jest osobna oś od `market_entry_units`. Gdy jest dodatnia, tamten
    /// limit nie ścina całego planu do głębokiej nogi: `market_entry_units`
    /// opisywał liczbę jednostek CAŁEGO sygnału, a ta oś świadomie rozdziela
    /// wykonanie na `teraz + cofnięcie`.
    #[serde(default = "default_market_hybrid_now_units")]
    pub market_hybrid_now_units: u32,
    /// Maksymalna liczba POZOSTAŁYCH jednostek pending w hybrydzie, liczona
    /// od najlepszej/najgłębszej krawędzi. 0 = wszystkie pozostałe.
    #[serde(default = "default_market_hybrid_pending_units")]
    pub market_hybrid_pending_units: u32,
    /// Mnożnik wolumenu jednostek otwieranych natychmiast przez hybrydę.
    /// Wartość <= 0 jest traktowana jak 1.0 (bez zmiany), aby zero w starym
    /// lub częściowym presecie nie wyłączyło po cichu wykonania.
    #[serde(default = "default_market_hybrid_lot_mult")]
    pub market_hybrid_lot_mult: f64,
    /// Maksymalna odległość bieżącej ceny od płytkiej krawędzi strefy, przy
    /// której wolno jeszcze wejść natychmiast. 0 = bez dodatkowego limitu.
    #[serde(default = "default_market_hybrid_max_chase_usd")]
    pub market_hybrid_max_chase_usd: f64,
    /// Cel nogi natychmiastowej: 0 = przydział planera; 1..254 = TP o takim
    /// numerze (z ograniczeniem do ostatniego istniejącego); 255 = bez TP.
    #[serde(default = "default_market_hybrid_tp_stage")]
    pub market_hybrid_tp_stage: u8,
    /// Sygnał BEZ `LIMITS`, który doszedł do wskazanego TP zanim wypełniła
    /// się jakakolwiek nasza pozycja, traci wszystkie niewypełnione pendingi.
    ///
    /// 0 = wyłącznie wspólne `pending_lifetime` (stary kontrakt); 1..254 =
    /// etap TP; 255 = nigdy nie kasuj tą osią. Jawne LIMITS są nietknięte.
    #[serde(default = "default_market_unfilled_cancel_stage")]
    pub market_unfilled_cancel_stage: u8,
    #[serde(default = "default_pending_cancel_on_riskfree")]
    pub pending_cancel_on_riskfree: bool,
    /// Bank CAŁOŚCI koszyka na etapie N celów (Pakiet B4). 0 = OFF.
    ///
    /// Tyler często bankuje CAŁE koszyki w strefie TP3 zamiast trzymać
    /// runnery (na 5049 +930 $, na 5037 −478 $ — niejednoznaczne, DO SWEEPA,
    /// nie do produkcji). Przy N > 0 `handle_tp_hit` na etapie >= N zamyka
    /// wszystkie pozycje koszyka, kasuje pendingi i kończy koszyk.
    /// 0 (domyślnie) = zachowanie dzisiejsze co do bitu.
    #[serde(default = "default_bank_all_at_stage")]
    pub bank_all_at_stage: u8,
    /// Confirm mandatory basket exits against broker positions AND pendings.
    /// Persist failed exit intent and retry at most once per second, without
    /// rebuilding the closing basket. False preserves the historical path.
    #[serde(default)]
    pub confirmed_exit_retry: bool,
    /// Account-level broker receipt reconciliation (not a strategy filter).
    /// Keeps position ownership after closure, deduplicates confirmed deal IDs,
    /// and reconciles volume from execution receipts/snapshots. OFF is legacy.
    #[serde(default)]
    pub close_receipt_reconcile: bool,
    /// RAM-only retention of new LIMIT entries during a temporary receipt barrier.
    /// Requires an explicit UTC receipt clock and verified execution session.
    #[serde(default)]
    pub defer_entry_until_receipts: bool,
    /// First receipt age (seconds); edits never extend it. Invalid/nonpositive
    /// values fail closed. Restart does not replay this RAM-only queue.
    #[serde(default = "default_deferred_entry_max_age_s")]
    pub deferred_entry_max_age_s: f64,
    #[serde(default = "default_stat_be_prog_usd")]
    pub stat_be_prog_usd: f64,
    /// timer: zamknij pozycję ~na BE jeśli wisi X min bez zysku (0 = OFF)
    pub oae_timeout_min: f64,
    pub oae_profit_min: f64,

    pub oae_skip_after_riskfree: bool,
    pub no_tp_after_stage: u8,
    pub no_reenter_from_stage: u8,
    pub day_gate_od_salda: f64,
    /// Bramki dobowe działają tylko PONIŻEJ tego salda otwarcia doby (`0` = bez ograniczenia).
    ///
    /// Odwrotność [`Settings::day_gate_od_salda`]: chroń dobę, póki konto jest
    /// małe i jedna zła seria potrafi je skasować, a po przekroczeniu progu
    /// puść je bez sufitu. Oba pola można łączyć — wtedy bramka działa
    /// wyłącznie w przedziale sald między nimi.
    ///
    /// `0` znaczy BRAK GÓRNEGO OGRANICZENIA, czyli parytet.
    pub day_gate_do_salda: f64,
    /// Nie dokładaj re-entry do koszyka, który jest już zabezpieczony.
    ///
    /// Kiedy kanał mówi „risk free", ogłasza koniec budowania pozycji:
    /// zwija gorsze wejścia i zostawia jedno z ochroną. Dokładanie po tym
    /// komunikacie odbudowuje ekspozycję, którą kanał właśnie zdjął.
    ///
    /// W zapisie z rachunku bot dołożył PIĘĆ wejść po „RISK FREE" —
    /// 4403.29, 4403.59, 4403.89, 4404.27, 4404.59 — i wszystkie pięć
    /// weszło do trzynastki zamkniętej po stracie parę minut później.
    pub reenter_stop_after_riskfree: bool,

    // ---------- breakeven / trailing ----------
    /// SL na wejście po +X pkt zysku (0 = OFF)
    pub be_lock_pts: f64,
    /// SL na wejście dla WSZYSTKICH po trafieniu TP1
    pub be_at_tp1: bool,
    #[serde(default)]
    pub be_od_etapu: u8,
    #[serde(default)]
    pub be_min_pozycji: u32,
    #[serde(default)]
    pub cele_pomin_za_cena: bool,
    pub entry_jeden_na_glebokiej: bool,
    pub sl_po_tp1_na_krawedz: bool,
    pub sl_wlasny_na_pozycje: f64,
    /// o ile powyżej wejścia stawiać BE (pokrycie spreadu)
    pub be_offset: f64,
    /// Automatyczna propozycja BE (etap TP / SET BE / kanał RISK FREE)
    /// nigdy nie cofa lepszego istniejącego SL, także bez trailingu S/R.
    /// Nie zmienia jawnego MOVE SL ani trybu `riskfree_runner_stop=Off`.
    /// OFF = historyczne nadpisanie; nie oznacza wyłączenia istniejącej
    /// niezależnej zapadki rodziny S/R. To ochrona ceny SL, nie certyfikat
    /// zerowego wyniku koszyka po kosztach, poślizgu i dawnych zamknięciach.
    pub be_never_loosen: bool,
    #[serde(default)]
    pub be_covers_late_fills: bool,
    pub trail_mode: TrailMode,
    pub trail_start: f64,
    pub trail_gap: f64,
    pub trail_lock_pct: f64,
    pub trail_tiers: String,
    /// osobny, luźniejszy trailing dla N najlepszych pozycji
    pub trail_split: bool,
    pub trail_runners_n: u32,
    pub trail_runner_mode: TrailMode,
    pub trail_runner_start: f64,
    pub trail_runner_gap: f64,
    pub trail_runner_lock_pct: f64,
    pub trail_runner_tiers: String,
    /// nie wysyłaj SL bliżej ceny niż tyle (broker odrzuci)
    pub trail_min_dist: f64,
    /// SL = osiągnięty TP[etap − lag] ± oddech
    pub ladder_from_tp: usize,
    pub ladder_lag: usize,
    pub ladder_offset: f64,
    /// Schodkowy SL zależny od rangi pozycji w koszyku.
    pub smart_sl_mode: SmartSlMode,
    /// O ile etapów opóźnić cały łańcuch SMART SL (1 = nic po TP1, ruch od TP2).
    pub smart_sl_delay: usize,
    /// Schodkowy SL działa dopiero na koszyku ZABEZPIECZONYM (po RISK FREE
    /// albo „SECURING PARTIAL PROFITS").
    ///
    /// Tak działał ten przełącznik w poprzednim bocie: drabinka stopów była
    /// nagrodą za komunikat sygnalisty, a nie zachowaniem domyślnym. Przed
    /// zabezpieczeniem koszyk zostaje na stop-lossie z sygnału.
    pub smart_sl_only_after_rf: bool,
    /// Po RISK FREE / SPP podłogą SMART SL jest breakeven, nie SL sygnału.
    ///
    /// Bez tego runner „zabezpieczony" komunikatem mógł wrócić pod cenę
    /// wejścia, bo łańcuch liczył się od pierwotnego stop-lossa.
    pub smart_sl_floor_be_after_rf: bool,
    /// Ponawiaj modyfikacje SL/TP odrzucone przez brokera co tyle sekund.
    ///
    /// Broker odrzuca SL/TP zbyt blisko ceny (stops level) albo przy
    /// requote. Jedna próba i cisza oznacza, że trailing po prostu ZNIKA —
    /// dokładnie tak gubił się w poprzednim bocie. 0 = bez ponawiania.
    pub sltp_retry_s: f64,

    #[serde(default = "default_trail_sr_enabled")]
    pub trail_sr_enabled: bool,
    #[serde(default = "default_trail_sr_scope")]
    pub trail_sr_scope: TrailSrScope,
    #[serde(default = "default_trail_sr_activation")]
    pub trail_sr_activation: TrailSrActivation,
    /// Próg $ zysku od ceny wejścia (na uncji) dla wariantu `Gain`.
    /// **[M: czytane tylko przy `trail_sr_activation = Gain`]**.
    /// 0 = od wejścia (równoważne `Entry`).
    #[serde(default = "default_trail_sr_min_gain")]
    pub trail_sr_min_gain: f64,
    #[serde(default = "default_trail_sr_min_dist_price")]
    pub trail_sr_min_dist_price: f64,

    #[serde(default)]
    pub entry_warstwy_offset: f64,
    /// CZY SLUCHAC TRESCI. Przy `true` przesuniecie warstw podane w sygnale
    /// („ADDING 3 PIPS TO EACH LIMIT ORDER BELOW THE FIRST ENTRY") ma
    /// pierwszenstwo przed `entry_warstwy_offset`. Sygnal, ktory NIC nie
    /// mowi, zachowuje sie dokladnie jak dotad — reagujemy wylacznie na to,
    /// co napisane. `false` (domyslnie) = kontrakt zera.
    #[serde(default)]
    pub entry_warstwy_z_tekstu: bool,
    #[serde(default)]
    pub runner_cele_n: u32,
    /// Odstep miedzy kolejnymi celami runnera w dolarach (kanon: ~10).
    #[serde(default = "runner_krok_domyslny")]
    pub runner_cele_krok: f64,
    /// Ile procent POZOSTALEJ pozycji inkasowac na kazdym celu runnera.
    /// Tyler bral ~10 %. 0 = nie inkasuj, biegnij dalej.
    #[serde(default)]
    pub runner_partial_pct: f64,
    #[serde(default = "sr_tf_min_domyslne")]
    pub trail_sr_tf_min: u32,
    /// Swing = ekstremum z `n` swiecami ostro gorszymi po obu stronach
    /// (dawniej stale 3). Wieksze `n` = rzadsza, mocniejsza struktura.
    #[serde(default = "sr_fractal_domyslny")]
    pub trail_sr_fractal_n: u32,
    /// O ile ZA poziom chowamy stop (dawniej stale 0,5).
    #[serde(default = "sr_offset_domyslny")]
    pub trail_sr_offset: f64,
    /// Oddech przed NASTEPNYM nieodhaczonym celem sygnalu (dawniej stale 2,0).
    #[serde(default = "sr_min_dist_tp_domyslny")]
    pub trail_sr_min_dist_tp: f64,
    /// Ile godzin potwierdzonej struktury trzymamy wstecz (dawniej stale 24).
    #[serde(default = "sr_okno_h_domyslne")]
    pub trail_sr_struct_window_h: u32,
    /// Minimalna jakość potwierdzonego swinga, liczona jako jego lokalna
    /// prominencja / ATR z WYŁĄCZNIE zamkniętych świec. 0 = brak filtra.
    #[serde(default)]
    pub trail_sr_min_prominence_atr: f64,
    /// Dynamiczny oddech stopu za strukturą: `ATR * mnożnik`.
    /// Skuteczny offset jest maksimum z offsetu stałego i obu dynamicznych.
    /// 0 = nie czytaj ATR dla offsetu.
    #[serde(default)]
    pub trail_sr_offset_atr_mult: f64,
    /// Dynamiczny oddech stopu za strukturą: referencyjny spread (p90 z
    /// zamkniętych świec) * mnożnik. 0 = nie czytaj spreadu dla offsetu.
    #[serde(default)]
    pub trail_sr_offset_spread_mult: f64,
    /// Okno ATR i spreadu. Czytane tylko, gdy co najmniej jedna z trzech osi
    /// dynamicznych wyżej jest dodatnia.
    #[serde(default = "sr_atr_period_domyslny")]
    pub trail_sr_atr_period: u32,

    // ---------- wyjścia ----------
    /// zamknij po cofnięciu o X % szczytu (0 = OFF)
    pub harvest_retrace_pct: f64,
    pub harvest_start: f64,
    /// zamknij pozycję w zysku, która przez X min nie zrobiła nowego szczytu
    pub stale_take_min: f64,
    pub stale_take_profit: f64,
    pub stale_take_min2: f64,
    pub stale_take_profit2: f64,
    /// REVERSAL-EXIT: gwałtowne odwrócenie przy wysokiej zmienności.
    ///
    /// Warunek: w oknie `rev_exit_window_min` zakres H−L ≥ `rev_exit_range`
    /// ORAZ cena przesunęła się o co najmniej `rev_exit_slope` PRZECIW
    /// pozycji. Wtedy bankujemy wszystko, co ma zysk ≥ `rev_exit_profit`.
    ///
    /// W poprzednim bocie ta funkcja liczyła klucz cache z `len()` pełnego
    /// bufora — a ten po 12 h zamarzał na stałej wartości i reguła cicho
    /// przestawała działać. Tutaj okno liczy się z czasu zdarzenia, więc
    /// taki błąd jest niemożliwy.
    pub rev_exit_range: f64,
    pub rev_exit_slope: f64,
    pub rev_exit_profit: f64,
    pub rev_exit_window_min: f64,
    /// RE-ENTRY: po trafionym celu wejdź ponownie, gdy cena wróci do strefy.
    ///
    /// „TP1 HIT AGAIN AFTER PULLING BACK" — tak prowadzi koszyk sygnalista
    /// i tak wygrywały wszystkie zwycięskie presety poprzedniego bota.
    pub reenter_after_tp: bool,
    pub reenter_min_tp_stage: usize,
    pub reenter_max: u32,

    // ---------- bramki wejść ----------
    pub session_filter: bool,
    /// CO filtruje okno godzin — patrz [`SesjaBramka`].
    #[serde(default = "sesja_bramka_domyslna")]
    pub sesja_bramka: SesjaBramka,
    /// godziny czasu serwera, np. "7-20" albo "7-11,13-20"
    pub session_hours: String,
    pub max_open_positions: u32,

    pub enforce_position_limit_on_fill: bool,
    #[serde(default)]
    pub limit_kasuje_tylko_nadmiar: bool,
    pub max_open_baskets: u32,

    pub exposure_bonus_profit_pct: f64,
    /// O ile pozycji wolno wtedy przekroczyć `max_open_positions`.
    pub exposure_bonus_positions: u32,
    /// O ile koszyków wolno wtedy przekroczyć `max_open_baskets`.
    pub exposure_bonus_baskets: u32,

    pub exposure_count_pendings: bool,
    /// maks. łączny wolumen w jedną stronę (0 = bez limitu)
    pub max_directional_lots: f64,
    pub streak_pause_n: u32,
    pub streak_pause_min: f64,
    pub signal_filter: bool,
    pub side_filter: SideFilter,
    pub skip_tags: String,
    pub require_tags: String,
    pub regime_filter: RegimeFilter,
    pub regime_ma_hours: f64,
    /// Po jakiej cenie oceniać reżim — patrz [`RegimeCena`].
    #[serde(default = "regime_cena_domyslna")]
    pub regime_cena: RegimeCena,
    /// Czym jest próg reżimu — patrz [`RegimeMiara`].
    #[serde(default = "regime_miara_domyslna")]
    pub regime_miara: RegimeMiara,
    /// Kasuj niewypełnione limity, gdy reżim przestał je przepuszczać.
    ///
    /// Bez tego zlecenie przepuszczone dziś wypełnia się jutro w zupełnie
    /// innym reżimie i nikt tego nie sprawdza. `false` = parytet.
    #[serde(default)]
    pub regime_pilnuj_limitow: bool,
    /// Percentyl okna dla [`RegimeMiara::Percentyl`] (0–100). `50` = mediana.
    #[serde(default = "regime_percentyl_domyslny")]
    pub regime_percentyl: f64,
    /// MARTWA STREFA wokół progu, w dolarach. `0` = OFF.
    ///
    /// Bez niej decyzja przeskakuje z „wpuszczam" na „blokuję" przy ruchu
    /// o centa wokół progu — bot szarpie, a każde szarpnięcie kosztuje spread.
    /// W martwej strefie filtr NIE ODMAWIA: brak wyraźnego sygnału to nie
    /// jest sygnał przeciwny.
    #[serde(default)]
    pub regime_strefa_martwa: f64,
    /// DRUGIE OKNO odniesienia w godzinach. `0` = OFF (jedno okno).
    ///
    /// Jedno okno nie odróżnia chwilowego cofnięcia w trendzie od odwrócenia
    /// trendu — do tego trzeba dwóch horyzontów. Przy włączonym drugim oknie
    /// sygnał przechodzi tylko wtedy, gdy OBA okna się zgadzają.
    #[serde(default)]
    pub regime_okno2_h: f64,
    /// Dolny próg zmienności okna (zakres max−min w dolarach), poniżej którego
    /// filtr MILCZY. `0` = OFF.
    ///
    /// W rynku bez ruchu położenie ceny wobec progu jest szumem, a nie
    /// informacją o reżimie — blokowanie na tej podstawie to kara za losowość.
    #[serde(default)]
    pub regime_zmiennosc_min: f64,
    /// Górny próg zmienności okna, powyżej którego filtr MILCZY. `0` = OFF.
    ///
    /// Przy rozrywającym ruchu (news, luka) próg policzony z poprzednich dni
    /// opisuje rynek, którego już nie ma. Wtedy uczciwiej jest nie mieć zdania
    /// niż mieć nieaktualne.
    #[serde(default, alias = "regime_range_mute_usd")]
    pub regime_zmiennosc_max: f64,
    /// Co robić po przekroczeniu `regime_zmiennosc_max` — patrz [`RegimeGdyRozerwany`].
    #[serde(
        default = "regime_gdy_rozerwany_domyslny",
        alias = "regime_range_mute_mode"
    )]
    pub regime_gdy_rozerwany: RegimeGdyRozerwany,

    #[serde(default)]
    pub regime_soft: bool,
    /// Mnożnik liczby szczebli siatki w miękkim reżimie (1.0 = bez zmiany).
    /// Wynik zaokrąglany w dół, ale nigdy poniżej jednego szczebla — koszyk
    /// bez szczebla to koszyk, którego nie ma, a to jest twarda bramka pod
    /// inną nazwą.
    #[serde(default = "jeden_f64")]
    pub regime_soft_units_mult: f64,
    /// Mnożnik lota w miękkim reżimie (1.0 = bez zmiany). Wchodzi w
    /// `lot_size` PRZED sufitem lota, dokładnie tak jak mnożnik zmienności:
    /// sufit jest polisą na ścianę marginesu i nie ma prawa ustąpić przed
    /// regułą rozmiaru.
    #[serde(default = "jeden_f64")]
    pub regime_soft_lot_mult: f64,
    /// Limit pozycji naraz obowiązujący w miękkim reżimie (0 = bez zmiany).
    /// Osobne pole, a nie mnożnik, bo limit pozycji jest liczbą całkowitą
    /// o twardym znaczeniu i „0,7 pozycji" nie znaczy nic.
    #[serde(default)]
    pub regime_soft_max_positions: u32,
    #[serde(default)]
    pub sanity_zone_max: f64,
    /// Maksymalna odległość NAJDALSZEGO celu od strefy, w dolarach. `0` = brak.
    ///
    /// Łapie literówkę w cyfrze celu — `TP2 3065` zamiast `3965` przy
    /// sprzedaży z 3975 daje cel oddalony o 906 $. Taka pozycja nigdy nie
    /// zamknie się na TP i wisi do wygaśnięcia koszyka albo do stopu.
    /// NOVA ma cele 3-20 $ od strefy, więc próg rzędu 60 $ odsiewa literówki
    /// i nie rusza żadnego prawdziwego celu.
    #[serde(default)]
    pub sanity_tp_max: f64,
    /// Czy cele muszą być coraz DALEJ od strefy (TP1 < TP2 < TP3 < TP4).
    ///
    /// Kolejność celów jest w tych kanałach regułą bez wyjątku, więc jej
    /// złamanie znaczy błąd w treści, a nie zamysł. Wyłapuje też przypadki,
    /// w których literówka nie jest skrajna i sam próg odległości by ją
    /// przepuścił.
    #[serde(default)]
    pub sanity_tp_rosnace: bool,
    /// Czy KAŻDY cel musi leżeć po stronie zysku (kupno powyżej, sprzedaż poniżej).
    ///
    /// Cel po złej stronie to dla silnika cel osiągnięty w chwili wejścia —
    /// zamyka transzę natychmiast po cenie wejścia i inkasuje sam spread.
    #[serde(default)]
    pub sanity_tp_strona: bool,
    /// Mnożnik LIMITU RYZYKA KOSZYKA w miękkim reżimie (1.0 = bez zmiany).
    ///
    /// To jest właściwe pokrętło rozmiaru, a `regime_soft_lot_mult` bywa
    /// bezużyteczne — i warto wiedzieć dlaczego. Gdy `risk_per_basket_pct > 0`
    /// (FS-M3-SYN ma 50), plan siatki jest PRZESKALOWYWANY do zadanego procentu
    /// kapitału. Lot bazowy skraca się wtedy w rachunku: przemiar z mnożnikiem
    /// 0,05 i 3,0 dał wyniki IDENTYCZNE co do centa, bo obie siatki zostały
    /// sprowadzone do tego samego ryzyka. Mnożnik lota działa wyłącznie
    /// w presetach BEZ limitu ryzyka koszyka.
    #[serde(default = "jeden_f64")]
    pub regime_soft_risk_mult: f64,
    #[serde(default)]
    pub slhit_pause_n: u32,
    /// Długość pauzy hamulca SL-HIT w minutach. `0` = do końca doby serwera.
    #[serde(default)]
    pub slhit_pause_min: f64,
    #[serde(default)]
    pub slhit_pause_lot_mult: f64,

    // ---------- ochrona kapitału ----------
    pub max_dd_pct: f64,
    pub max_dd_usd: f64,
    pub dd_guard_scope: DdGuardScope,

    pub max_portfolio_risk_pct: f64,
    /// Obsunięcie (%) od bazy `dd_guard_scope`, powyżej którego budżet ryzyka
    /// nowego koszyka mnożymy przez `dd_soft_mult` (0 = wyłączone).
    pub dd_soft_pct: f64,
    /// Mnożnik budżetu powyżej `dd_soft_pct`. 0,5 = graj połową.
    pub dd_soft_mult: f64,
    /// Drugi, głębszy próg dławika (0 = wyłączony).
    pub dd_hard_pct: f64,
    /// Mnożnik budżetu powyżej `dd_hard_pct`. 0,25 = graj ćwiartką.
    pub dd_hard_mult: f64,

    // ---------- REGUŁY DOŚWIADCZONEGO TRADERA ----------
    //
    // Warstwa decyzji, których nie da się wyrazić przez procent szczytu ani
    // przez czas. Każda odwzorowuje konkretne zachowanie człowieka przy
    // wykresie. Wszystkie domyślnie WYŁĄCZONE — nie zmieniają istniejących
    // presetów.
    /// Nie zamykaj żadną z reguł wyjścia przez tyle minut od otwarcia.
    /// Reguły są najbardziej zawodne tuż po wejściu, gdy szczyt jest jeszcze
    /// mały i procent liczony od niego jest czystym szumem.
    pub exit_min_hold_min: f64,
    /// Nie zamykaj regułą wyjścia poniżej takiego zysku — chroni przed
    /// „braniem zysku" mniejszego niż spread i prowizja razem wzięte.
    pub exit_min_profit: f64,
    /// Zamknij, gdy zysk osiągnie tę wielokrotność własnego ryzyka
    /// (|wejście − SL|). Trader mówi „mam 3R, biorę"; wielokrotność ryzyka
    /// normalizuje decyzję między sygnałami o różnej szerokości stopa —
    /// 3 $ przy stopie 1 $ to co innego niż przy stopie 9 $. 0 = wyłączone.
    pub exit_r_multiple: f64,
    /// Zamknij CAŁY koszyk, gdy jego łączny wynik przekroczy tę kwotę.
    /// Przy siatce kilku wejść pojedyncza pozycja nic nie znaczy — decyzja
    /// dotyczy koszyka, a silnik dotąd zamykał wyłącznie per pozycja.
    pub basket_target_usd: f64,
    /// Zamknij, gdy cena zbliży się na tyle dolarów do okrągłego poziomu
    /// będącego wielokrotnością `exit_round_step`. Złoto reaguje na pełne
    /// dziesiątki i setki; to jedyna cecha strukturalna policzalna bez
    /// interpretacji wykresu. 0 = wyłączone.
    pub exit_round_dist: f64,
    pub exit_round_step: f64,
    /// Zamknij, gdy spread urośnie ponad tę wielokrotność swojej mediany.
    /// Spread jest jedynym kosztem tej strategii i jedyną obserwowalną miarą
    /// płynności w tickach — jego skok zapowiada droższe wyjście. 0 = wyłączone.
    pub exit_spread_mult: f64,
    /// Zamknij koszyk, gdy z kanału przyjdzie sygnał w przeciwnym kierunku.
    /// To najsilniejsza informacja, jaką kanał wysyła, a silnik ją ignorował.
    pub exit_on_opposite_signal: bool,
    /// Strefa sygnalu PRZECIWNEGO staje sie CELEM dla otwartych pozycji.
    ///
    /// Patrz [`CelZPrzeciwnego`]. To jest inna reguła niż
    /// `exit_on_opposite_signal`: tamta zamyka po rynku, ta USTAWIA TP
    /// i czeka, aż cena sama tam dojdzie.
    #[serde(default = "cel_z_przeciwnego_domyslny")]
    pub cel_z_przeciwnego: CelZPrzeciwnego,
    /// Margines pod krawędzią strefy przeciwnej ($). Inkasujemy odrobinę
    /// PRZED nadawcą, bo to on swoim zleceniem tworzy tam podaż.
    #[serde(default)]
    pub cel_z_przeciwnego_zapas: f64,
    /// Wstrzymaj wszystkie reguły wyjścia na tyle minut po komunikacie
    /// o trafionym celu. Trader widzi „TP2 HIT" i wie, że ruch ma rozpęd —
    /// nie zamyka na pierwszej korekcie. 0 = wyłączone.
    pub hold_after_tp_hit_min: f64,

    // ---------- MĄDRE WYJŚCIE ----------
    //
    // Odwzorowanie tego, co człowiek robi patrząc na wykres:
    //  * „widzę duży zysk — zamykam",
    //  * „widzę, że nagle zaczęło spadać — zamykam z mniejszym, ale zyskiem",
    //  * „spada, ALE tuż pod ceną mam niezrealizowany limit — zostawiam, bo
    //    prawdopodobnie wróci, a po drodze dokupię taniej".
    //
    // Trzeci warunek jest tu najważniejszy i nie ma go w żadnej klasycznej
    // regule wyjścia: pozycja spadająca w stronę WŁASNEJ siatki limitów to
    // zupełnie inna sytuacja niż pozycja spadająca w próżnię. W pierwszym
    // przypadku spadek obniża średnią cenę koszyka i powrót wyprowadza całość
    // na plus; w drugim jest tylko stratą.
    /// Włącza mądre wyjście. Domyślnie wyłączone — nie zmienia zachowania
    /// istniejących presetów.
    pub smart_exit: bool,
    /// Zamknij natychmiast, gdy zysk pozycji przekroczy tę kwotę (0 = nigdy).
    pub smart_exit_take: f64,
    /// Zamknij, gdy zysk cofnie się o ten ułamek szczytu (0,30 = oddane 30 %).
    pub smart_exit_giveback: f64,
    /// …ale dopiero od takiego szczytu zysku — inaczej reguła ścinałaby
    /// pozycje, które ledwo weszły na plus.
    pub smart_exit_min_peak: f64,
    /// Zamknij, gdy cena spada szybciej niż tyle dolarów na minutę,
    /// licząc z ostatnich `smart_exit_speed_window_s` sekund (0 = nie patrz).
    pub smart_exit_drop_speed: f64,
    pub smart_exit_speed_window_s: f64,
    /// **Serce reguły.** NIE zamykaj, jeżeli w odległości tylu dolarów pod
    /// ceną (dla kupna; nad ceną dla sprzedaży) czeka niezrealizowany limit
    /// tego samego koszyka. 0 = nie uwzględniaj siatki.
    pub smart_exit_hold_if_pending: f64,
    /// Ile najwyżej niezrealizowanych limitów pod ceną wystarcza, żeby trzymać.
    /// Zero oznacza „dowolna liczba" — pole istnieje, bo jeden czekający limit
    /// to inna sytuacja niż pięć.
    pub smart_exit_min_pendings: u32,
    /// Czyje limity się liczą.
    ///
    /// Limit z TEGO SAMEGO koszyka obniża średnią cenę tej pozycji i powrót
    /// wyprowadza całość na plus — to argument najmocniejszy. Limit z INNEGO
    /// koszyka nie uśrednia nam nic, ale nadal jest miejscem, w którym ktoś
    /// (my) będzie kupował — czyli wsparciem dla ceny. Dlatego oba warianty
    /// mają sens i różnią się siłą.
    pub smart_exit_pending_scope: PendingScope,
    /// Limity BLIŻSZE niż tyle dolarów są pomijane w tej ocenie.
    ///
    /// Limit tuż pod ceną i tak zaraz się wypełni, więc nie niesie informacji
    /// „cena ma dokąd wrócić" — niesie ją dopiero taki, do którego rynek musi
    /// jeszcze kawałek zejść. Bez tego progu reguła myliłaby jedno z drugim.
    pub smart_exit_pending_min_dist: f64,
    pub day_target_usd: f64,
    pub day_target_close: bool,
    pub day_target_scale_lot: bool,
    pub day_trail_stop_usd: f64,
    /// wszystkie progi dolarowe mnożone przez lot/0.01
    pub usd_scale_with_lot: bool,
    /// zamknij wszystko o godzinie X (0 = OFF)
    pub eod_flat_hour: f64,
    pub flat_weekend: bool,
    pub flat_weekend_hour: f64,
    /// twarda podłoga equity — poniżej niej bot nie otwiera nic nowego
    pub equity_floor_pct: f64,


    pub riskfree_enabled: bool,
    /// Próg zysku KOSZYKA (zrealizowany + otwarty) w dolarach, po którym
    /// reguła się uruchamia. 0 = nie wyzwalaj kwotą.
    pub riskfree_trigger_usd: f64,
    /// Ten sam próg wyrażony w wielokrotności RYZYKA koszyka, czyli
    /// Σ |cena wejścia − SL| × 100 × wolumen. 0 = nie wyzwalaj wielokrotnością.
    /// Gdy ustawione są oba, wystarczy spełnić którykolwiek.
    pub riskfree_trigger_r: f64,
    pub riskfree_keep_units: u32,
    /// Margines stopu runnera ponad średnią ważoną cenę wejścia koszyka.
    /// Dodatni = stop odrobinę W ZYSKU (pokrywa spread wyjścia), ujemny =
    /// odrobinę luzu, żeby szpilka nie zbierała runnera przed ruchem.
    pub riskfree_be_offset: f64,
    /// Co runner dostaje jako cel po uwolnieniu koszyka od ryzyka.
    /// `NoTpTrailOnly` = leci bez celu, wyłącznie na stopie i trailingu.
    pub riskfree_runner_target: RiskFreeRunnerTarget,
    /// Co runner dostaje jako STOP. Patrz [`RiskFreeRunnerStop`] — stop
    /// przyklejony do breakeven jest punktem odniesienia, nie zwycięzcą.
    pub riskfree_runner_stop: RiskFreeRunnerStop,
    pub riskfree_runner_gap: f64,
    pub riskfree_runner_max_hold_min: f64,

    pub basket_max_age_min: f64,

    pub fast_fill_reject_s: f64,

    pub fast_fill_layers: u32,

    pub fast_fill_soft_age_min: f64,

    pub zone_exit_adverse_s: f64,

    /// Co zrobić po trwałym wyjściu przeciw: `false` = skasuj same limity
    /// (nie uśredniaj dalej), `true` = zamknij też pozycje.
    ///
    /// Domyślnie tryb miękki, bo rodzina `fast_fill` pokazała mechanicznie,
    /// że twarde przerwanie płaci koszt wyjścia podwodnych warstw w połowie
    /// ruchu przeciw nam.
    pub zone_exit_adverse_close: bool,

    pub reenter_min_return_s: f64,

    pub pyramid_after_stage: u32,

    /// Mnożnik lota dokładki piramidy (1.0 = tyle samo co warstwa bazowa).
    ///
    /// ⚠ Przy 2,0 najniższe equity spada do 39 $ — pod bitter spotem.
    /// Nie przekraczać 1,5.
    pub pyramid_lot_mult: f64,

    pub pyramid_regime_lookback: u32,

    /// Maksymalny udział koszyków-przelotów (w procentach), przy którym
    /// piramida jeszcze wolno działa.
    pub pyramid_regime_max_fast_pct: f64,

    pub fast_addon_move_usd: f64,
    pub fast_addon_window_s: f64,
    /// Ile dokładek tempowych wolno dołożyć do JEDNEGO koszyka.
    pub fast_addon_max: u32,
    /// Mnożnik lota dokładki wobec warstwy bazowej.
    pub fast_addon_lot_mult: f64,
    /// Minimalny etap TP koszyka wymagany do dokładki (0 = bez wymogu).
    /// Przy 1 reguła zbliża się do piramidy, ale nadal wchodzi rynkiem.
    pub fast_addon_min_stage: u32,
    /// Minimalny odstęp między dokładkami tego samego koszyka (sekundy).
    /// Bez niego jeden gwałtowny ruch odpala wszystkie dozwolone dokładki
    /// w kilku kolejnych tickach.
    pub fast_addon_cooldown_s: f64,

    pub pyramid_min_equity_mult: f64,

    pub trend_filter_enabled: bool,
    /// Okno odniesienia w godzinach (np. 24 = doba, 168 = tydzień).
    pub trend_filter_window_h: f64,
    /// Próg zmiany ceny w PROCENTACH, od którego trend uznajemy za
    /// przeciwny sygnałowi. 0,5 = „złoto spadło o pół procent w oknie".
    pub trend_filter_drop_pct: f64,
    /// Co zrobić z sygnałem idącym pod trend.
    pub trend_filter_mode: TrendFilterMode,
    /// Mnożnik liczby jednostek w wariancie miękkim (0,5 = połowa rozmiaru).
    pub trend_filter_shrink: f64,

    pub pending_drop_require_zone_touch: bool,

    // ---------- `trail_runners_n` — MARTWE POLE, TERAZ NAPRAWDĘ DZIAŁA ----------
    //
    // Forensyka: `trail_runners_n = 1` i `= 3` dawały wynik identyczny do
    // szóstego miejsca po przecinku. Powód: „runner" był definiowany jako
    // pozycja BEZ take-profitu, więc ich liczba nie zależała od tego pola
    // w ogóle. Pole kłamało użytkownikowi panelu.
    /// Runnerem do luźniejszego trailingu jest N NAJLEPSZYCH WEJŚĆ koszyka
    /// (dla kupna: najniższa cena), a nie „pozycja bez celu". Dopiero wtedy
    /// `trail_runners_n` cokolwiek znaczy. Odpowiednik `_trail_plan`
    /// z `bot.py:7635`.
    pub trail_runners_by_depth: bool,

    pub entry_weights_from_rr: bool,
    /// Wykładnik nagięcia wag R:R. 1,0 = wprost proporcjonalnie,
    /// 0,5 = pierwiastek (łagodniej), 2,0 = kwadrat (ostrzej).
    pub entry_weights_rr_power: f64,
    /// Najwyższy dopuszczalny stosunek największej wagi do najmniejszej.
    /// Bez sufitu szczebel o R:R 8,0 dostawałby szesnastokrotność szczebla
    /// o R:R 0,50 i cały koszyk zawisłby na jednym poziomie.
    pub entry_weights_rr_cap: f64,

    #[serde(default)]
    pub drop_unplaceable_levels: bool,

    #[serde(default)]
    pub zakaz_ponizej_krawedzi: bool,

    pub adaptive_params: bool,
    /// `sl_min_dist` = mnożnik × szerokość strefy Z SYGNAŁU (przed offsetami).
    /// 0 = nie licz z szerokości.
    pub sl_min_dist_zone_mult: f64,
    /// `sl_min_dist` = mnożnik × zakres H−L z okna `adaptive_atr_window_min`.
    /// 0 = nie licz ze zmienności. Gdy działają oba źródła, wygrywa WIĘKSZE:
    /// stop ma być dość szeroki dla obu powodów naraz.
    pub sl_min_dist_atr_mult: f64,
    /// Podłoga i sufit wyliczonego `sl_min_dist` (0 = bez ograniczenia).
    /// Bez nich sygnał ze strefą 0,5 $ dostałby stop niemożliwy do przyjęcia.
    pub sl_min_dist_floor: f64,
    pub sl_min_dist_cap: f64,
    /// `entry_deep_offset` = mnożnik × szerokość strefy z sygnału (0 = off).
    /// To jest dźwignia GŁĘBOKOŚCI: rozciąga strefę w stronę lepszych wejść
    /// proporcjonalnie do tego, jak szeroki setup podał sygnalista.
    pub entry_deep_zone_mult: f64,
    /// Szerokość strefy uznana za „typową". Liczba szczebli siatki rośnie
    /// i maleje proporcjonalnie do `szerokość / ta wartość`. 0 = off.
    pub entry_units_zone_ref: f64,
    /// Okno (minuty) zastępczego ATR — zakres H−L bufora zmienności.
    pub adaptive_atr_window_min: f64,
    /// Mnożnik liczby jednostek wg PORY DNIA, „7-11:0.5,15-17:2".
    /// Puste = bez różnicowania. Godziny w czasie serwera brokera.
    pub units_by_hour: String,

    pub rearm_grid_on_return: bool,
    /// Zachowaj pusty koszyk, który już handlował, aż cena będzie mogła
    /// uruchomić `rearm_grid_on_return`.
    ///
    /// `false` zachowuje historyczne sprzątanie Rust 1:1: pusty koszyk starszy
    /// niż 60 sekund przechodzi w `Done`, nawet gdy powrót ceny miałby go
    /// później przezbroić. `true` działa wyłącznie razem z
    /// `rearm_grid_on_return` i tylko dla koszyka z `had_positions` — nie
    /// przedłuża życia sygnałów, które nigdy nie dostały wypełnienia.
    pub rearm_keep_empty_alive: bool,
    pub rearm_block_after_secured: bool,
    /// Komunikat SPP blokuje późniejsze przezbrojenie także wtedy, gdy w
    /// chwili komunikatu koszyk jest już płaski.
    ///
    /// To nie ustawia fałszywego `secured`: płaski koszyk nie ma ryzyka ani
    /// stopu do zabezpieczenia. Zapamiętuje wyłącznie jawne veto ponownego
    /// wejścia (np. Synergy: „DO NOT ... ENTER THE MARKET AGAIN”).
    /// `false` zachowuje dotychczasową ścieżkę Rust co do bitu.
    pub spp_blocks_rearm_when_flat: bool,
    /// Minimalny łączny wynik koszyka (zrealizowany + otwarty) w dolarach,
    /// przy którym wolno dokładać. To jest owo „potwierdzenie": dokładamy do
    /// setupu, który już działa, a nie do przegranej pozycji.
    pub rearm_min_basket_profit: f64,
    /// Ile razy jeden koszyk może się przezbroić (0 = bez limitu).
    pub rearm_max_times: u32,
    pub rearm_bez_pozycji: bool,
    /// Sufit wieku koszyka uspionego, w godzinach. Po nim setup uznajemy za
    /// nieaktualny — sygnal sprzed dwoch dni opisuje inny rynek. 0 = bez
    /// sufitu (odradzane: koszyk zyje wtedy do `basket_max_age_min`).
    pub rearm_bez_pozycji_max_h: f64,
    /// Najkrótszy odstęp między przezbrojeniami (minuty).
    pub rearm_min_gap_min: f64,

    pub day_target_pct: f64,
    /// Stop dnia jako PROCENT SZCZYTU equity dnia (0 = off).
    /// Liczony od szczytu, nie od salda otwarcia — chroni zysk, który już
    /// był na rachunku, a nie tylko kapitał startowy.
    pub day_trail_stop_pct: f64,
    /// Stop dnia uzbraja się dopiero, gdy dzień był na plusie o tyle procent
    /// equity otwarcia (0 = uzbrojony od razu). Bez tego progu „stop dnia"
    /// przy pierwszym normalnym obsunięciu kończy dobę, zanim cokolwiek
    /// zarobi.
    pub day_trail_arm_pct: f64,

    pub daily_signal_budget: u32,
    /// Najmniejsze akceptowane R:R sygnału, liczone w miejscu realistycznego
    /// wejścia (gorsza krawędź strefy): |TP1 − krawędź| / |krawędź − SL|.
    /// 0 = bez progu. To jest ranking sygnałów sprowadzony do jednej liczby,
    /// którą da się policzyć W CHWILI sygnału, bez zaglądania w przyszłość.
    pub signal_min_rr: f64,
    /// Pasmo akceptowanej szerokości strefy (0 = bez ograniczenia).
    /// Strefa zbyt wąska nie daje głębokości, zbyt szeroka oznacza, że
    /// sygnalista sam nie wie, gdzie jest poziom.
    pub signal_min_zone_width: f64,
    pub signal_max_zone_width: f64,

    pub merge_same_side: bool,
    /// Jak stary może być koszyk, żeby nowy sygnał uznać za jego ciąg dalszy.
    pub merge_window_min: f64,
    /// Wymagany udział pokrycia stref (0,5 = połowa węższej z nich).
    pub merge_min_overlap: f64,

    pub exit_via_limit: bool,
    /// Ile ponad drugą stronę spreadu żądać dodatkowo (0 = sam spread).
    pub exit_limit_offset: f64,
    /// Jak długo czekać, zanim wyjdziemy awaryjnie po rynku (sekundy).
    /// To jest cena tej reguły: cofnięcie w oknie oczekiwania zabiera
    /// więcej, niż wynosi zaoszczędzony spread.
    pub exit_limit_wait_s: f64,
    /// Minimalny zysk pozycji (w cenie), poniżej którego nie kombinujemy
    /// i wychodzimy od razu po rynku.
    pub exit_limit_min_profit: f64,

    // ---------- broker ----------
    /// minimalna odległość SL/TP od ceny akceptowana przez brokera
    pub stops_level: f64,
    /// prowizja za lot (Vantage Standard STP = 0)
    pub commission_per_lot: f64,

    pub swap_enabled: bool,
    /// Punkty za noc dla pozycji DŁUGIEJ (ujemne = koszt).
    pub swap_long_points: f64,
    /// Punkty za noc dla pozycji KRÓTKIEJ (dodatnie = przychód).
    pub swap_short_points: f64,
    /// Wartość jednego punktu swapowego w dolarach na LOTA.
    /// Dla XAUUSD u Vantage: 1 punkt = 1,00 $, więc 0,01 lota BUY kosztuje
    /// −0,7582 $ za noc.
    pub swap_point_value: f64,
    pub swap_rollover_weekday: u32,
    /// Mnożnik w dniu potrójnego naliczenia.
    pub swap_rollover_mult: f64,
    #[serde(default)]
    pub swap_pomijaj_weekend: bool,
    #[serde(default)]
    pub swap_rollover_z_serwera: bool,
    #[serde(default = "trzy_u32")]
    pub swap_rollover3days_mt5: u32,
    #[serde(default)]
    pub runner_ksiegowanie_v2: bool,
    /// Jedynym źródłem zrealizowanego PnL koszyka jest `Broker::drain_closed`.
    /// Wyłącza dziewięć ręcznych dopisań po komendach zamknięcia, które
    /// następny tick księgował ponownie z raportu brokera. Nie deduplikuje
    /// po tickecie: kolejne częściowe zamknięcia mają osobne wykonania.
    /// Wynik pojawia się przy następnym drenażu w `on_tick`, nie przed
    /// otrzymaniem potwierdzonego wykonania. Nie przelicza starych migawek.
    /// `false` zachowuje historyczne księgowanie i wyniki backtestów.
    #[serde(default)]
    pub basket_realized_broker_only: bool,
    /// Canonical closed-net from complete cost receipts, not source-defined PnL.
    /// Requires basket_realized_broker_only and an active supported broker cost
    /// pipeline. OFF preserves legacy cashflow, metadata and profit semantics.
    /// Does NOT migrate stored realized state or certify live/history completeness.
    #[serde(default)]
    pub closed_profit_net_costs: bool,
    /// Account/runtime opt-in: restore verified pending SL/TP, exit deadlines
    /// and the latched day stop. Incomplete restore holds NEW risk only.
    /// This is not a durable receipt or atomic multi-file checkpoint contract.
    #[serde(default)]
    pub restore_strategy_continuation: bool,
    #[serde(default)]
    pub msg_kurs_sprzed_luki: bool,
    /// Ścisła kolejność paczki zdarzeń w adapterze live.
    ///
    /// Most MT5 potrafi zwrócić kilka ticków naraz. Stara pętla ustawia w
    /// brokerze ostatnią kwotę paczki, obsługuje wiadomości, a dopiero potem
    /// odtwarza wcześniejsze ticki. W efekcie `on_tick(stary_q)` widzi przez
    /// `Broker::quote()` przyszły kurs. `true` odtwarza ticki po kolei,
    /// synchronizuje kwotę brokera z każdym z nich i dopiero potem obsługuje
    /// wiadomości. Tę samą kolejność stosują `runner` oraz `okna` w
    /// backteście i `In_LiveTickOrderStrict` w CONDUIT_XT. `false` zachowuje
    /// historyczną kolejność każdej ścieżki, nie jest deklaracją ich parytetu.
    #[serde(default)]
    pub live_tick_order_strict: bool,

    pub slippage_pending_pts: f64,

    pub sim_margin_check_on_fill: bool,

    #[serde(default)]
    pub sim_validate_pending_stops: bool,

    #[serde(default)]
    pub reenter_respect_cap: bool,

    #[serde(default)]
    pub sl_edit_reaches_pendings: bool,

    #[serde(default)]
    pub honor_stop_orders: bool,

    #[serde(default)]
    pub hint_veto: bool,

    #[serde(default)]
    pub reply_veto: bool,

    pub sync_only_live_levels: bool,

    #[serde(default)]
    pub tp_correction_to_broker: bool,

    #[serde(default)]
    pub runner_max_hold_rule_only: bool,

    #[serde(default)]
    pub runner_max_hold_bez_reguly: bool,

    /// Z-10, druga połowa: czy `SECURING PARTIAL PROFITS` uzbraja zegar
    /// runnera tak samo jak `RISK FREE`.
    ///
    /// Dwa komunikaty o tej samej semantyce miały dwa różne zachowania: RF
    /// ustawiał `secured_ts` (koszyk łapał się na limit trzymania), SPP
    /// ustawiał samo `secured` (nie łapał się nigdy).
    ///
    /// Domyślnie WYŁĄCZONE.
    #[serde(default)]
    pub spp_arms_runner_clock: bool,

    #[serde(default)]
    pub tp_hit_match_level: bool,

    #[serde(default)]
    pub tp_unindexed_pips_require_price: bool,

    /// PriceOnly rejects every Telegram TpHit branch, including unindexed
    /// PIPS. Explicit RF/SL/SPP management is unchanged. OFF preserves the
    /// historical Rust bypass; native CONDUIT_XT already has this gate.
    #[serde(default)]
    pub tp_price_only_strict: bool,

    #[serde(default)]
    pub rf_level_sanity_max_usd: f64,

    /// F5: zapamiętuj każdą jednoznacznie zaadresowaną wiadomość zarządzającą
    /// jako alias koszyka, aby odpowiedzi wielopoziomowe dziedziczyły adresata.
    /// Domyślnie wyłączone dla zgodności ze starymi presetami.
    #[serde(default)]
    pub reply_graph_transitive: bool,

    /// Poziom marginu, przy którym broker zaczyna zamykać pozycje (%).
    ///
    /// MT5 zamyka NAJBARDZIEJ STRATNĄ pozycję pojedynczo i przelicza poziom,
    /// a nie wszystko naraz. Przy siatce wielu małych pozycji to zupełnie
    /// inny przebieg — a dotyczy jedynego progu bezwzględnego, jaki mamy:
    /// „konto nie może zostać wyzerowane".
    pub stop_out_level_pct: f64,
    #[serde(default)]
    pub sim_margin_at_market: bool,
    /// Poziom wezwania do uzupełnienia depozytu (%). Powyżej stop outu;
    /// broker nie zamyka, ale nie przyjmuje nowych zleceń.
    pub margin_call_level_pct: f64,
    pub server_tz_offset_ms: i64,
    /// Ile dodać do znacznika wiadomości, żeby trafić w zegar ticków.
    ///
    /// `None` = użyj `server_tz_offset_ms` (poprawne, gdy eksport Telegrama
    /// jest w UTC — tak jest dla naszego `signals.json`). Wartość jawna
    /// przydaje się, gdy źródło wiadomości ma własną strefę.
    ///
    /// BEZ TEGO PRZESUNIĘCIA BACKTEST DAJE DARMOWY ZYSK: silnik dostaje
    /// sygnał, a razem z nim strumień cen sprzed trzech godzin, więc może
    /// wejść po kursie z przeszłości, znając już strefę i cele. Objawia się
    /// to zawyżoną realizacją limitów i nierealnym PF.
    pub msg_clock_offset_ms: Option<i64>,
    /// modelowane opóźnienie od wiadomości do zlecenia (ms)
    pub exec_latency_ms: i64,
    pub slippage_pts: f64,

    // ---------- AI ----------
    pub ai_enabled: bool,
    /// Czy model AI ZASTĘPUJE całe zarządzanie pozycjami.
    ///
    /// `true` (dotychczasowe, jedyne zachowanie): włączenie AI wyłącza
    /// trailing, stagnację, żniwo, BE-lock, mądre wyjście i regułę RISK FREE.
    /// To jest ZAMIERZONE — model ma prowadzić pozycję sam — ale nazwa
    /// `ai_enabled` tego nie mówiła: w panelu wyglądała na „dodaj AI",
    /// a znaczyła „zdejmij wszystkie zabezpieczenia".
    ///
    /// `false` pozwala prowadzić model RÓWNOLEGLE z regułami, co jest
    /// właściwym trybem, dopóki model nie bije presetów w dolarach.
    pub ai_replaces_management: bool,
    pub ai_model: String,
    /// co ile sekund model podejmuje decyzję
    pub ai_decision_interval_s: f64,

    pub mt5_autostart: bool,
    /// Pilnuj terminala i wznawiaj go po wyłączeniu.
    pub mt5_watchdog: bool,
    /// Pełna ścieżka do `terminal64.exe`. Puste = wykryj automatycznie
    /// (rejestr, `Program Files`, katalogi `%APPDATA%\MetaQuotes\Terminal`).
    pub mt5_terminal_path: String,
    /// Ile prób połączenia w jednej serii, zanim bot przejdzie do długiego
    /// czekania. `bot.py` miał tu 10 na sztywno.
    pub mt5_retry_attempts: u32,
    /// Odstęp między próbami w serii (sekundy).
    pub mt5_retry_delay_s: f64,
    /// Po ilu nieudanych próbach W SERII restartować aplikację MT5.
    ///
    /// 1 = pierwsza próba jest „na sucho", restart dopiero przed drugą.
    /// 0 = nigdy nie restartuj (tylko próbuj się łączyć).
    ///
    /// `bot.py` restartował aplikację przed KAŻDĄ próbą, także pierwszą —
    /// przez co chwilowa zadyszka terminala kosztowała pełny cykl
    /// zamknij-zabij-uruchom zamiast jednego ponowienia.
    pub mt5_restart_after: u32,
    /// Co ile sekund sprawdzać, czy terminal nadal żyje.
    pub mt5_health_interval_s: f64,

    // ---------- dziennik zdarzeń (JSON Lines) ----------
    //
    // Strumień maszynowo czytelny, obok dotychczasowego logu tekstowego.
    // Powstał z listy braków wytkniętych logowi `bot.py`: brak daty w
    // nagłówku, zamknięcie bez wolumenu, brak powiązania pozycja ↔ koszyk ↔
    // wiadomość, brak stanu przed decyzją, `EMERGENCY_STOP` bez treści,
    // `TARGET_ignore` bez powodu i 266 sekcji `email_sent` zagłuszających dane.
    /// Czy w ogóle zapisywać dziennik. Wyłączony nie kosztuje ANI JEDNEJ
    /// alokacji — poziom sprawdzany jest przed zbudowaniem treści zdarzenia.
    pub journal_enabled: bool,
    /// Najniższy zapisywany poziom. `Debug` zapisuje także wiadomości, które
    /// nic nie zmieniły; `Info` to rozsądny stan domyślny.
    pub journal_min_level: crate::journal::EventLevel,
    /// Dołączać migawkę stanu (bid/ask/spread/equity/margines/ekspozycja) do
    /// każdej decyzji. Bez niej nie da się ocenić, czy decyzja była słuszna.
    pub journal_snapshots: bool,
    /// Mierzyć maksymalne korzystne i niekorzystne wychylenie ceny (MFE/MAE)
    /// każdej pozycji. To jedyne źródło odpowiedzi na „czy dało się zamknąć
    /// lepiej", czyli na raport „ile pieniędzy zostawiono na stole".
    pub journal_excursions: bool,
    /// Pisać obok pliku `.jsonl` lustrzany `.log` dla oka.
    pub journal_text_mirror: bool,
    /// Po ilu dobach kasować stare pliki dziennika (0 = nigdy).
    pub journal_retention_days: u32,
    /// Ile zdarzeń wolno trzymać w buforze rdzenia między zrzutami na dysk.
    pub journal_buffer_cap: u32,

    // ---------- BRAMKI KAPITAŁOWE (rodzina `*_small`) ----------
    //
    // Wzorzec wzięty z `pyramid_min_equity_mult`, gdzie się sprawdził, i
    // uogólniony: mechanizm, który jest NIEPROPORCJONALNIE KOSZTOWNY na małym
    // koncie, dostaje drugą wartość obowiązującą DOPÓKI konto nie urośnie.
    //
    // Wspólna semantyka całej rodziny:
    //   * `X_small_mult <= 0`  → bramka WYŁĄCZONA, obowiązuje samo `X`
    //     (wynik musi być identyczny co do centa z przebiegiem bez bramki —
    //     to jest kontrola);
    //   * `saldo < saldo_startowe * X_small_mult` → obowiązuje `X_small`;
    //   * wyżej → obowiązuje `X`.
    //
    // ⚠ Odniesieniem jest saldo STARTOWE PRZEBIEGU, nie bieżące. Wobec
    // bieżącego warunek byłby tożsamościowo prawdziwy i całe pole nie robiłoby
    // NIC — a to jest dokładnie ta awaria konfiguracji, którą wykrywa
    // `martwe_ustawienia` (wynik identyczny co do dolara przy każdej wartości).
    //
    // ⚠ W trybie „każdy dzień osobno" saldo startowe wraca co dobę do kwoty
    // wyjściowej, więc bramka z progiem ≥ 2 praktycznie nigdy nie otwiera się
    // w ciągu dnia: mierzy wtedy „mechanizm w wariancie `_small` przez cały
    // czas", a nie działanie samej bramki. Bramki mają sens wyłącznie
    // w compoundingu — i tam trzeba je oceniać medianą ansamblu perturbacji,
    // bo pojedynczy compounding jest losowaniem.
    /// Liczba szczebli siatki obowiązująca na małym koncie.
    /// Wartość jest dociskana do ≥ 1 przez miejsca użycia.
    pub entry_units_small: u32,
    /// Próg (wielokrotność salda startowego) dla `entry_units_small`.
    pub entry_units_small_mult: f64,

    /// Limit ryzyka koszyka (% kapitału) obowiązujący na małym koncie.
    /// 0 = limit wyłączony, tak samo jak w polu bazowym.
    pub risk_per_basket_pct_small: f64,
    /// Próg dla `risk_per_basket_pct_small`.
    pub risk_per_basket_pct_small_mult: f64,

    /// Limit ponownych wejść obowiązujący na małym koncie (0 = bez limitu).
    pub reenter_max_small: u32,
    /// Próg dla `reenter_max_small`.
    pub reenter_max_small_mult: f64,

    /// Limit pozycji naraz obowiązujący na małym koncie (0 = bez limitu).
    pub max_open_positions_small: u32,
    /// Próg dla `max_open_positions_small`.
    pub max_open_positions_small_mult: f64,

    /// Limit żywych koszyków obowiązujący na małym koncie (0 = bez limitu).
    pub max_open_baskets_small: u32,
    /// Próg dla `max_open_baskets_small`.
    pub max_open_baskets_small_mult: f64,

    /// Twardy limit wieku koszyka obowiązujący na małym koncie (0 = brak).
    pub basket_max_age_min_small: f64,
    /// Próg dla `basket_max_age_min_small`.
    pub basket_max_age_min_small_mult: f64,

    /// Skrócone życie koszyka-przelotu (tryb miękki filtra tempa) na małym
    /// koncie. 0 = tryb miękki wyłączony, czyli filtr działa twardo.
    pub fast_fill_soft_age_min_small: f64,
    /// Próg dla `fast_fill_soft_age_min_small`.
    pub fast_fill_soft_age_min_small_mult: f64,

    /// Odstęp kolejnych wejść rynkowych i re-entry na małym koncie.
    pub market_entry_step_small: f64,
    /// Próg dla `market_entry_step_small`.
    pub market_entry_step_small_mult: f64,

    /// Minimalna odległość SL obowiązująca na małym koncie.
    pub sl_min_dist_small: f64,
    /// Próg dla `sl_min_dist_small`.
    pub sl_min_dist_small_mult: f64,

    /// Wielkość pozycji w procentach kapitału na małym koncie.
    ///
    /// Odwrotny kierunek niż reszta rodziny: przy 200 $ lot i tak leży na
    /// podłodze 0,01, więc obniżanie procentu nic nie robi — sens ma tylko
    /// PODNIESIENIE go, żeby konto szybciej wyszło ponad podłogę.
    pub lot_percent_small: f64,
    /// Próg dla `lot_percent_small`.
    pub lot_percent_small_mult: f64,

    pub lot_max_z_salda: f64,

    pub trail_atr_mult: f64,

    // ---------- ROZMIAR STEROWANY ZMIENNOŚCIĄ (sześć pól, czytać razem) ----------
    //
    // Wszystkie sześć jest domyślnie WYŁĄCZONYCH i przy `vol_size_mode = Off`
    // silnik nie wykonuje ani jednej dodatkowej operacji zmiennoprzecinkowej
    // w `lot_size()`. Parytet jest tu strukturalny, nie „numeryczny".
    /// Który wzór na mnożnik rozmiaru. `Off` = wyłączone.
    pub vol_size_mode: VolSizeMode,

    /// DOCELOWY ZASIĘG w USD dla trybu [`VolSizeMode::Target`].
    ///
    /// Mnożnik = `vol_size_target / zmienność_odsezonowana`, gdzie zmienność
    /// to zakres H−L z okna `adaptive_atr_window_min` (60 min u obu mistrzów),
    /// podzielony przez sezonowy mnożnik swojej godziny serwera.
    ///
    /// Wartość odczytać wprost: przy 12,0 rynek ruszający się o 12 $ na
    /// godzinę daje lot bazowy, przy 24 $/h — połowę, przy 6 $/h — podwójny.
    /// Punkt odniesienia z danych: średnia godzina XAUUSD w tym pliku to
    /// 16,5 $ zakresu, a wewnątrz sesji 8-16 około 14 $.
    ///
    /// `0` = wyłączone (mnożnik 1,0), niezależnie od trybu.
    pub vol_size_target: f64,

    /// DOLNA KLAMRA mnożnika. `0` = brak klamry (obowiązuje twardy próg 0,05,
    /// żeby dzielenie przez rozdmuchaną zmienność nie wyzerowało lota).
    ///
    /// Klamry są tu ważniejsze niż w literaturze, bo my nie mamy dźwigni
    /// dowolnej: przy 300 $ krok lota to 0,01, a lot bazowy 0,02 — mnożnik
    /// 0,5 i 0,74 dają ten sam wolumen. Zbyt szeroka klamra dolna nie tyle
    /// zmniejsza pozycję, co ją WYŁĄCZA (spadek do `lot_min`).
    pub vol_size_min_mult: f64,

    /// GÓRNA KLAMRA mnożnika. `0` = brak klamry (twardy sufit 20,0).
    ///
    /// Sufit `lot_max` i tak przycina każde zlecenie osobno, więc ta klamra
    /// pilnuje czegoś innego: żeby cicha noc nie kazała botu wejść pełnym
    /// kontem w rynek, który za dwie godziny ruszy z danymi makro.
    pub vol_size_max_mult: f64,

    pub vol_size_percentile_okno: u32,

    pub vol_size_odsezonuj: bool,

    // ---------- KREDYT BONUSOWY ----------
    // MT5 API raportuje Balance i Credit oddzielnie; przy braku pozycji
    // Equity = Balance + Credit. Stary model wliczał kredyt w Balance.
    /// Oddzielny ACCOUNT_CREDIT poza ACCOUNT_BALANCE (kontrakt MT5).
    ///
    /// OFF zachowuje historyczny model 1:1. ON: Balance jest surowym saldem
    /// bez bonusu; odlicz_kredyt odejmuje bonus tylko od Equity, a MinOfBoth
    /// liczy min(Balance, Equity - skuteczny kredyt). W symulacji kredyt
    /// powiększa Equity/free margin, nigdy Balance ani zysk zamknięty.
    #[serde(default)]
    pub credit_balance_separate: bool,
    pub odlicz_kredyt: bool,
    /// Kwota kredytu wpisana RĘCZNIE. **`0` = AUTOMAT** (bierz `ACCOUNT_CREDIT`
    /// z terminala); wartość dodatnia nadpisuje odczyt brokera.
    ///
    /// # Konwencja zera — czytać, zanim się to zmieni
    ///
    /// `0` znaczy tu „bez ręcznego nadpisania", tak samo jak `0` w pułapach
    /// globalnych znaczy „bez limitu". To NIE jest „kredyt wynosi zero".
    /// Ta sama konwencja odwrotnie odczytana kosztowała nas już realny
    /// rozjazd: `reenter_max = 0` czytane jako „wyłączone" zamiast „bez
    /// limitu". Wyłączenie `odlicz_kredyt` pomija odliczenie przy sizingu;
    /// NIE oznacza usunięcia faktycznego kredytu z rachunku.
    ///
    /// # Dlaczego domyślny jest AUTOMAT, a nie kwota wpisana przez człowieka
    ///
    /// Bonusy bywają zdejmowane: przy pierwszej wypłacie, po terminie
    /// promocji, przy zmianie warunków. Kwota wpisana ręcznie i zapomniana
    /// znaczy, że po zdjęciu bonusu bot nadal pomniejsza podstawę Equity
    /// (w starym modelu także Balance) — i gra za małym lotem, bez objawu poza
    /// wynikiem gorszym, niż powinien. Automat sam zejdzie do zera razem
    /// z bonusem.
    ///
    /// Groźniejszy jest błąd odwrotny — nie odjąć kredytu, którego terminal
    /// nie raportuje — dlatego pole ręczne istnieje w ogóle. Ale gdy oba
    /// źródła mówią różne rzeczy, panel to POKAZUJE jako ostrzeżenie
    /// (`rest.rs` / karta lota), zamiast po cichu wybrać jedno.
    pub kredyt_reczny: f64,

    pub parser_geometryczny: bool,
    /// Próg jakości odczytu geometrycznego, 0,0–1,0.
    ///
    /// Odczyt niosący pewność NIŻSZĄ niż próg jest odrzucany i wiadomość
    /// zostaje `Info`. Pewność to suma wag pięciu niezmienników: jednoznaczne
    /// słowo BUY/SELL (0,20), słowo kierunku stojące przy cenie (0,15), trzy
    /// grupy poziomów ze co najmniej dwoma celami (0,20), zgodność odczytu
    /// z tekstu z odczytem z geometrii (0,20) oraz sensowność odległości do
    /// stopu i do celów (2 × 0,125).
    ///
    /// # Konwencja zera
    ///
    /// `0` znaczy „bierz każdy odczyt, który w ogóle powstał" — a nie
    /// „wyłączone". Wyłącza `parser_geometryczny = false`. Przy wyłączonym
    /// przełączniku wartość tego pola nie jest w ogóle czytana.
    pub parser_min_pewnosc: f64,

    #[serde(default)]
    pub ea_enabled: bool,
    #[serde(default)]
    pub ea_tick_s: f64,
    /// Źródło sygnału maszyny stanu (patrz [`EaStateSrc`]).
    #[serde(default = "default_ea_state_src")]
    pub ea_state_src: EaStateSrc,
    /// Próg wejścia w OBRONĘ (w R albo % equity wg `ea_state_src`).
    /// **`0` = obrona nigdy** — to nie jest „obrona przy zerowej stracie".
    #[serde(default)]
    pub ea_defense_enter: f64,
    /// Próg wyjścia z obrony — musi być MNIEJ dotkliwy niż `_enter`
    /// (histereza dwustronna). `0` = z obrony nie wychodzimy progiem.
    #[serde(default)]
    pub ea_defense_exit: f64,
    /// Próg wejścia w AGRESJĘ. **`0` = agresja nigdy.**
    ///
    /// AGRESJA jest stanem POJEDYNCZEGO KOSZYKA, nie portfela: zwycięzca
    /// biegnie sam i nie licencjonuje kolegów (asymetria zasięgu z kanonu
    /// Synergy — strata nigdy nie podnosi ryzyka całości).
    #[serde(default)]
    pub ea_offense_enter: f64,
    /// Próg wyjścia z agresji. `0` = z agresji nie wychodzimy progiem.
    #[serde(default)]
    pub ea_offense_exit: f64,
    /// MINIMALNY CZAS TRWANIA STANU (anty-migotanie), sekundy. `0` = bez wymogu.
    ///
    /// Asymetryczny z założenia i to jest wiążące: **zaciskanie działa
    /// natychmiast, luzowanie wymaga przetrzymania.** Ochrona nie czeka na
    /// zegar — ta sama zasada co w `ea_ladder_scope` i w arbitrze.
    #[serde(default)]
    pub ea_state_dwell_s: f64,
    /// Zapadka stanu (patrz [`EaRatchet`], niezmiennik N10).
    #[serde(default = "default_ea_state_ratchet")]
    pub ea_state_ratchet: EaRatchet,
    /// Zapisywać każdą zmianę stanu z powodem i wartościami wejść.
    ///
    /// Domyślnie `true`, bo dziennik warstwy nie zmienia ANI JEDNEJ decyzji
    /// (bufor w pamięci rdzenia z twardym sufitem), a bez niego nie da się
    /// powiedzieć, dlaczego stan przełączył się w danej minucie.
    #[serde(default = "default_ea_state_journal")]
    pub ea_state_journal: bool,
    #[serde(default)]
    pub ea_dozor_sl: bool,

    #[serde(default)]
    pub ea_lot_z_wolnego_marginesu: f64,
    /// **A2 — STOP DOKŁADKOM PRZY STRACIE KOSZYKA.**
    ///
    /// `0` = **oś wyłączona** (nie „stop przy zerowej stracie").
    ///
    /// Jednostka idzie za `ea_state_src`: `FloatR` = pływająca strata koszyka
    /// podzielona przez jego R pierwotne, `FloatPctEquity` = ta strata jako
    /// % equity. Koszyk, którego pływający wynik zejdzie poniżej `−próg`,
    /// przestaje dostawać DOKŁADKI: `fast_addon_*`, `rearm_*`, `reenter_*`
    /// i piramidę. **Nowych koszyków oś nie dotyka** — pokrycie jest święte
    /// (`EA_DYNAMICZNE_SPEC` §4 B1: „NIE blokuje nowych koszyków").
    ///
    /// Zamyka jedną konkretną klasę zachowań: dokładanie do pozycji, która
    /// właśnie idzie pod wodę — czyli uśrednianie w dół pod inną nazwą.
    #[serde(default)]
    pub ea_stop_dokladek_przy_stracie: f64,
    /// **A2 — PRÓG POWROTU** (histereza dwustronna).
    ///
    /// `0` = brak osobnego progu powrotu, weto zdejmuje się na tym samym
    /// poziomie, na którym się zatrzasnęło. Wartość dodatnia MUSI być MNIEJ
    /// dotkliwa niż `ea_stop_dokladek_przy_stracie` — inaczej histereza
    /// działa w drugą stronę i weto zdejmuje się natychmiast po założeniu.
    ///
    /// Asymetria jest ta sama, co w maszynie stanu EA-CORE: zaciśnięcie
    /// (weto) działa NATYCHMIAST, zdjęcie wymaga powrotu do progu wyjścia.
    #[serde(default)]
    pub ea_stop_dokladek_powrot: f64,
    /// **A3 — REDUKCJA JEDNOSTEK PRZY ZAGĘSZCZENIU** (nachylenie na koszyk).
    ///
    /// `0` = **oś wyłączona**. Mnożnik liczby jednostek NOWEGO koszyka:
    /// `mult = clamp(1 − nachylenie × liczba_żywych_koszyków, podłoga, 1)`.
    ///
    /// Dziesiąty koszyk w tej samej godzinie jest inną decyzją niż pierwszy,
    /// choćby każdy z osobna mieścił się w swoim limicie ryzyka: portfel
    /// dzieli JEDEN margines i JEDEN dystans do ruiny. `max_open_baskets`
    /// odpowiada na to skokowo (jest miejsce / nie ma), ta oś — liniowo.
    #[serde(default)]
    pub ea_redukcja_przy_zageszczeniu: f64,
    /// **A3 — PODŁOGA MNOŻNIKA ZAGĘSZCZENIA**, zakres `0…1`.
    ///
    /// Poniżej tej wartości mnożnik nie zejdzie, więc oś nie zamienia się
    /// w twardą blokadę wejść przy dużej liczbie koszyków. Niezależnie od
    /// podłogi plan zachowuje co najmniej JEDNĄ jednostkę — siatka o zerowej
    /// liczbie szczebli to koszyk, którego nie ma, czyli blokada pod inną
    /// nazwą (ta sama reguła co w `regime_soft_units_mult`).
    #[serde(default)]
    pub ea_zageszczenie_podloga: f64,
    /// **A4 — STAN DNIA** (patrz [`EaStanDnia`]). Domyślnie `Off`.
    #[serde(default = "default_ea_stan_dnia")]
    pub ea_stan_dnia: EaStanDnia,
    #[serde(default = "default_ea_stan_dnia_prog_sl")]
    pub ea_stan_dnia_prog_sl: u32,
    /// **A4 — MNOŻNIK JEDNOSTEK PO UZBROJENIU**, przycięty do `(0; 1]`.
    ///
    /// `1,0` (domyślnie) = stan dnia nie zmienia rozmiaru, działa wyłącznie
    /// twardy zakaz dokładek. Wartości `> 1` są PRZYCINANE do 1,0 w kodzie,
    /// a nie tylko odradzane w opisie — „ryzyko nie rośnie po stracie" jest
    /// niezmiennikiem tej osi.
    #[serde(default = "default_ea_stan_dnia_jednostki_mult")]
    pub ea_stan_dnia_jednostki_mult: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lot_mode_percent: false,
            lot_fixed: 0.01,
            lot_percent: 1.0,
            lot_scale_step: 0.0,
            lot_max: 100.0,
            lot_min: 0.01,
            order_volume_contract_v2: false,

            zone_offset_mode: ZoneOffsetMode::None,
            entry_hi_offset: 0.0,
            entry_lo_offset: 0.0,
            entry_deep_offset: 4.0,
            entry_deep_frac_to_sl: 0.0,
            entry_tol_offset: 0.3,
            only_limit_signals: false,
            auto_limit: true,
            ignore_old_after_min: 0.0,
            skip_if_sl_breached: true,
            max_chase_beyond_zone: 0.0,

            entry_units: 1,
            entry_units_limit: 0,
            ppm: 1.0,
            ppm_enabled: false,
            entry_risk_budget: 0.0,
            entry_tp1_budget: 0.0,
            entry_depth_curve: 1.0,
            entry_allowance_usd: 0.0,
            entry_allowance_units: 0,
            entry_weights: String::new(),
            entry_uklad: String::new(),
            entry_uklad_kotwica: "Ocalaly".into(),
            entry_krzywa_kotwica: "Ocalaly".into(),
            tp_drabinka_kotwica: "Ocalaly".into(),
            risk_per_basket_pct: 0.0,
            toucher_units: 0,
            toucher_tp_index: 1,
            toucher_tp_one_based: false,
            toucher_bands: String::new(),
            pending_lifetime: PendingLifetime::UntilTp1,
            pending_drop_on_target: true,
            pending_drop_arm: false,
            pending_ttl_h: 0.0,
            pending_ttl_from_basket: true,
            ppm_for_limits: true,
            grid_anchor_absolute: false,
            // F6/G1: true = stary warunek `units_for_level` co do bitu.
            units_per_level: default_units_per_level(),
            // U-BUG38R: true = mnożnik kraty łapie też strefę (stare
            // zachowanie co do bitu — kontrakt zera).
            units_per_level_zone: default_units_per_level_zone(),
            pending_cross_policy: PendingCrossPolicy::Market,
            ppm_for_market: false,
            market_entry_step: 1.0,
            // GridAtOnce = zachowanie sprzed rozdzielenia pól. Preset, który
            // nic o tym polu nie wie, musi dostać liczby co do centa te same.
            market_entry_mode: MarketEntryMode::GridAtOnce,

            vol_window_min: 0.0,
            vol_range_usd: 15.0,
            vol_units_mult: 0.7,
            pending_resize_on_vol: false,
            pending_resize_s: 30.0,
            pending_relot_on_balance: false,
            pending_relot_reconcile_target: false,
            pending_relot_topup: false,
            pending_relot_up: true,
            pending_relot_down: true,
            pending_relot_up_od_salda: 0.0,
            pending_relot_wg_planu: true,

            // WYŁĄCZONE. Bramka nieruszalności: przy `expo_cap_pct = 0`
            // `redukuj_ekspozycje` wychodzi pierwszą linijką, więc każdy
            // istniejący preset dostaje liczby co do centa te same.
            expo_cap_pct: 0.0,
            expo_cap_close: false,
            expo_cap_s: 0.0,
            expo_cap_ml_pct: 0.0,
            ml_licz_wiszace: false,
            ml_min_wejscie: 0.0,
            ml_min_warstwa: 0.0,
            ml_min_reentry: 0.0,
            ml_min_rearm: 0.0,
            ml_min_piramida: 0.0,
            ml_min_fast_addon: 0.0,
            ml_min_relot_up: 0.0,
            ml_min_drabina: 0.0,
            konto_dzwignia: 0.0,
            wiek_od_wypelnienia: false,
            pending_drop_grace_min: 0.0,
            pending_drop_grace_max_dist: 0.0,
            pending_drop_keep_n: 0,
            lot_base: PodstawaLota::Balance,

            sl_min_dist: 0.0,
            sl_max_dist: 0.0,
            entry_sl_dist_limit: 0.0,
            virtual_sl: false,
            virtual_sl_only_when_rejected: true,
            virtual_sl_all: false,
            vsl_eval_s: 0.0,
            vsl_broker_offset: 0.0,

            tp_schedule: TpSchedule::Ladder,
            scale_out_pct: 30.0,
            official_pct: [15.0, 30.0, 30.0, 20.0],
            official_counts: "1,1,1".into(),
            official_spp: false,
            assign_tp_per_position: true,
            tp_open_offset: 5.0,
            tp_freeze_after_ladder: true,
            tp_open_extra: false,
            tp_source: TpSource::Either,
            tp_price_tolerance: 0.30,
            tp_price_front_run_usd: 0.0,
            tp_signal_max_lead_s: 0.0,
            tp_signal_max_lag_s: 0.0,
            tp_stage_from_broker_fill: true,
            tp_hit_fill_stages: true,
            bank_rounding: BankRounding::Up,
            bank_from: BankFrom::Worst,
            bank_close_last: false,
            last_runner: LastRunner::Runner,
            partial_close: false,
            partial_min_lot: 0.02,
            partial_pct_od_pierwotnego: false,
            cele_na_ostatnim: false,
            retarget_respects_final_target: false,
            sl_polowa_od_konca: 0,
            sl_polowa_ulamek: 0.5,
            spp_max_age_h: 12.0,
            spp_keep_tp: false,
            spp_sl_mode: SppSlMode::Off,
            spp_sl_pad: 0.0,

            risk_free_mode: RiskFreeMode::CloseAllKeepNearest,
            risk_free_runners: 1,
            risk_free_runner_target: RiskFreeRunnerTarget::LastTp,
            risk_free_trail: false,
            risk_free_be_min_profit: 0.0,
            out_at_entry_mode: OutAtEntryMode::CloseAll,
            oae_pod_woda: OaePodWoda::NicNieRob,
            oae_band_pts: 1.0,
            sl_hit_mode: SlHitMode::CancelPendings,
            sl_hit_verify_tol: 0.0,
            honor_cancel: true,
            honor_close_all: true,
            close_all_scope: default_close_all_scope(),
            partials_wykonuj: false,
            parser_luz_interpunkcyjny: false,
            recap_guard: false,
            profit_update_telemetry_only: false,
            partials_pct: 0.0,
            honor_market_open: false,
            basket_hint_tolerance: 0.6,
            dedup_edited_signals: true,
            dedup_pelny_status: default_dedup_pelny_status(),
            edycja_wykonuje_reszte_akcji: default_edycja_wykonuje_reszte_akcji(),
            dedup_klucz_z_wartoscia: default_dedup_klucz_z_wartoscia(),
            edycja_sieroty_nie_otwiera: default_edycja_sieroty_nie_otwiera(),
            entry_idempotencja: default_entry_idempotencja(),
            entry_edit_geometry_v2: false,
            sr_warmup_exact_ticks: false,
            dedup_management_po_restarcie: false,
            rf_wymaga_wykonania: default_rf_wymaga_wykonania(),
            market_entry_units: default_market_entry_units(),
            market_hybrid_now_units: default_market_hybrid_now_units(),
            market_hybrid_pending_units: default_market_hybrid_pending_units(),
            market_hybrid_lot_mult: default_market_hybrid_lot_mult(),
            market_hybrid_max_chase_usd: default_market_hybrid_max_chase_usd(),
            market_hybrid_tp_stage: default_market_hybrid_tp_stage(),
            market_unfilled_cancel_stage: default_market_unfilled_cancel_stage(),
            pending_cancel_on_riskfree: default_pending_cancel_on_riskfree(),
            bank_all_at_stage: default_bank_all_at_stage(),
            confirmed_exit_retry: false,
            close_receipt_reconcile: false,
            defer_entry_until_receipts: false,
            deferred_entry_max_age_s: 300.0,
            stat_be_prog_usd: default_stat_be_prog_usd(),
            oae_timeout_min: 0.0,
            oae_profit_min: 0.5,
            oae_skip_after_riskfree: false,
            no_tp_after_stage: 0,
            no_reenter_from_stage: 0,
            day_gate_od_salda: 0.0,
            day_gate_do_salda: 0.0,
            reenter_stop_after_riskfree: false,

            be_lock_pts: 0.0,
            be_at_tp1: false,
            be_od_etapu: 0,
            be_min_pozycji: 0,
            cele_pomin_za_cena: false,
            entry_jeden_na_glebokiej: false,
            sl_po_tp1_na_krawedz: false,
            sl_wlasny_na_pozycje: 0.0,
            be_offset: 0.0,
            be_never_loosen: false,
            be_covers_late_fills: false,
            trail_mode: TrailMode::Off,
            trail_start: 25.0,
            trail_gap: 20.0,
            trail_lock_pct: 50.0,
            trail_tiers: "5:1,10:5,15:9,20:14,30:23,50:42".into(),
            trail_split: false,
            trail_runners_n: 1,
            trail_runner_mode: TrailMode::Tiered,
            trail_runner_start: 5.0,
            trail_runner_gap: 8.0,
            trail_runner_lock_pct: 50.0,
            trail_runner_tiers: "5:1,10:4,20:12,35:26,60:50,100:88".into(),
            trail_min_dist: 0.0,
            ladder_from_tp: 0,
            ladder_lag: 0,
            ladder_offset: 0.0,
            smart_sl_mode: SmartSlMode::Off,
            smart_sl_delay: 0,
            smart_sl_only_after_rf: false,
            smart_sl_floor_be_after_rf: true,
            sltp_retry_s: 3.0,

            // OS_SR: wyłączone = silnik bajt w bajt jak przed dodaniem pól.
            trail_sr_enabled: default_trail_sr_enabled(),
            trail_sr_scope: default_trail_sr_scope(),
            trail_sr_activation: default_trail_sr_activation(),
            trail_sr_min_gain: default_trail_sr_min_gain(),
            trail_sr_min_dist_price: default_trail_sr_min_dist_price(),
            entry_warstwy_offset: 0.0,
            entry_warstwy_z_tekstu: false,
            runner_cele_n: 0,
            runner_cele_krok: 10.0,
            runner_partial_pct: 0.0,
            trail_sr_tf_min: 1,
            trail_sr_fractal_n: 3,
            trail_sr_offset: 0.5,
            trail_sr_min_dist_tp: 2.0,
            trail_sr_struct_window_h: 24,
            trail_sr_min_prominence_atr: 0.0,
            trail_sr_offset_atr_mult: 0.0,
            trail_sr_offset_spread_mult: 0.0,
            trail_sr_atr_period: 14,

            harvest_retrace_pct: 0.0,
            harvest_start: 8.0,
            stale_take_min: 0.0,
            stale_take_profit: 15.0,
            stale_take_min2: 0.0,
            stale_take_profit2: 35.0,
            rev_exit_range: 0.0,
            rev_exit_slope: 14.0,
            rev_exit_profit: 4.0,
            rev_exit_window_min: 60.0,
            reenter_after_tp: false,
            reenter_min_tp_stage: 1,
            reenter_max: 0,

            sesja_bramka: SesjaBramka::Sygnal,
            session_filter: false,
            session_hours: "7-20".into(),
            max_open_positions: 0,
            enforce_position_limit_on_fill: false,
            limit_kasuje_tylko_nadmiar: false,
            max_open_baskets: 0,
            exposure_bonus_profit_pct: 0.0,
            exposure_bonus_positions: 0,
            exposure_bonus_baskets: 0,
            exposure_count_pendings: false,
            max_directional_lots: 0.0,
            streak_pause_n: 0,
            streak_pause_min: 60.0,
            signal_filter: false,
            side_filter: SideFilter::Both,
            skip_tags: String::new(),
            require_tags: String::new(),
            regime_filter: RegimeFilter::Off,
            regime_ma_hours: 72.0,
            regime_cena: RegimeCena::Rynkowa,
            regime_miara: RegimeMiara::Srednia,
            regime_pilnuj_limitow: false,
            regime_percentyl: 50.0,
            regime_strefa_martwa: 0.0,
            regime_okno2_h: 0.0,
            regime_zmiennosc_min: 0.0,
            regime_zmiennosc_max: 0.0,
            regime_gdy_rozerwany: RegimeGdyRozerwany::Milcz,
            regime_soft: false,
            regime_soft_units_mult: 1.0,
            regime_soft_lot_mult: 1.0,
            regime_soft_max_positions: 0,
            sanity_zone_max: 0.0,
            sanity_tp_max: 0.0,
            sanity_tp_rosnace: false,
            sanity_tp_strona: false,
            regime_soft_risk_mult: 1.0,
            slhit_pause_n: 0,
            slhit_pause_min: 0.0,
            slhit_pause_lot_mult: 0.0,

            max_dd_pct: 0.0,
            max_dd_usd: 0.0,
            dd_guard_scope: DdGuardScope::Daily,
            max_portfolio_risk_pct: 0.0,
            dd_soft_pct: 0.0,
            dd_soft_mult: 0.5,
            dd_hard_pct: 0.0,
            dd_hard_mult: 0.25,

            exit_min_hold_min: 0.0,
            exit_min_profit: 0.0,
            exit_r_multiple: 0.0,
            basket_target_usd: 0.0,
            exit_round_dist: 0.0,
            exit_round_step: 10.0,
            exit_spread_mult: 0.0,
            exit_on_opposite_signal: false,
            cel_z_przeciwnego: CelZPrzeciwnego::Off,
            cel_z_przeciwnego_zapas: 0.0,
            hold_after_tp_hit_min: 0.0,

            smart_exit: false,
            smart_exit_take: 0.0,
            smart_exit_giveback: 0.30,
            smart_exit_min_peak: 4.0,
            smart_exit_drop_speed: 0.0,
            smart_exit_speed_window_s: 60.0,
            smart_exit_hold_if_pending: 2.0,
            smart_exit_min_pendings: 1,
            smart_exit_pending_scope: PendingScope::SameBasket,
            smart_exit_pending_min_dist: 0.3,
            day_target_usd: 0.0,
            day_target_close: false,
            day_target_scale_lot: false,
            day_trail_stop_usd: 0.0,
            usd_scale_with_lot: false,
            eod_flat_hour: 0.0,
            flat_weekend: false,
            flat_weekend_hour: 20.0,
            equity_floor_pct: 0.0,

            riskfree_enabled: false,
            riskfree_trigger_usd: 0.0,
            riskfree_trigger_r: 0.0,
            riskfree_keep_units: 1,
            riskfree_be_offset: 0.0,
            riskfree_runner_target: RiskFreeRunnerTarget::LastTp,
            riskfree_runner_stop: RiskFreeRunnerStop::Be,
            riskfree_runner_gap: 15.0,
            // 90 minut, a nie 72 godziny: przewaga sygnału zmienia znak
            // po ok. 90 min (60 min +1 045 $, 90 min −321 $, 24 h −3 494 $)
            riskfree_runner_max_hold_min: 90.0,
            basket_max_age_min: 0.0,
            fast_fill_reject_s: 0.0,
            fast_fill_layers: 3,
            fast_fill_soft_age_min: 0.0,
            zone_exit_adverse_s: 0.0,
            zone_exit_adverse_close: false,
            reenter_min_return_s: 0.0,
            pyramid_after_stage: 0,
            pyramid_lot_mult: 1.0,
            pyramid_regime_lookback: 0,
            pyramid_regime_max_fast_pct: 30.0,
            pyramid_min_equity_mult: 0.0,
            fast_addon_move_usd: 0.0,
            fast_addon_window_s: 60.0,
            fast_addon_max: 1,
            fast_addon_lot_mult: 1.0,
            fast_addon_min_stage: 0,
            fast_addon_cooldown_s: 60.0,

            trend_filter_enabled: false,
            trend_filter_window_h: 24.0,
            trend_filter_drop_pct: 0.0,
            trend_filter_mode: TrendFilterMode::Shrink,
            trend_filter_shrink: 0.5,

            pending_drop_require_zone_touch: false,
            trail_runners_by_depth: false,

            entry_weights_from_rr: false,
            entry_weights_rr_power: 1.0,
            entry_weights_rr_cap: 4.0,
            drop_unplaceable_levels: false,
            // KONTRAKT ZERA: wyłączone = przebieg identyczny co do bitu.
            zakaz_ponizej_krawedzi: false,

            adaptive_params: false,
            sl_min_dist_zone_mult: 0.0,
            sl_min_dist_atr_mult: 0.0,
            sl_min_dist_floor: 0.0,
            sl_min_dist_cap: 0.0,
            entry_deep_zone_mult: 0.0,
            entry_units_zone_ref: 0.0,
            adaptive_atr_window_min: 60.0,
            units_by_hour: String::new(),

            rearm_grid_on_return: false,
            rearm_keep_empty_alive: false,
            rearm_block_after_secured: false,
            spp_blocks_rearm_when_flat: false,
            rearm_min_basket_profit: 0.0,
            rearm_max_times: 1,
            // USPIENIE KOSZYKA — domyslnie WYLACZONE, wiec przebieg jest
            // identyczny co do bitu z tym sprzed zmiany.
            rearm_bez_pozycji: false,
            rearm_bez_pozycji_max_h: 6.0,
            rearm_min_gap_min: 15.0,

            day_target_pct: 0.0,
            day_trail_stop_pct: 0.0,
            day_trail_arm_pct: 0.0,

            daily_signal_budget: 0,
            signal_min_rr: 0.0,
            signal_min_zone_width: 0.0,
            signal_max_zone_width: 0.0,

            merge_same_side: false,
            merge_window_min: 20.0,
            merge_min_overlap: 0.5,

            exit_via_limit: false,
            exit_limit_offset: 0.0,
            exit_limit_wait_s: 60.0,
            exit_limit_min_profit: 0.0,

            stops_level: 0.20,
            commission_per_lot: 0.0,

            // Wartości ODPYTANE Z SERWERA Vantage, nie z dokumentacji.
            swap_enabled: true,
            swap_long_points: -75.82,
            swap_short_points: 27.41,
            swap_point_value: 1.0,
            // 3 = CZWARTEK w konwencji poniedziałek=0. Broker potraja
            // rolowanie kończące środę, a taki deal ma znacznik czwartku.
            swap_rollover_weekday: 3,
            swap_rollover_mult: 3.0,
            // D1 / D1b: obie osie wyłączone — parytet co do centa
            swap_pomijaj_weekend: false,
            swap_rollover_z_serwera: false,
            swap_rollover3days_mt5: 3,
            // D5/D6/D4 i D3: wyłączone — parytet co do centa
            runner_ksiegowanie_v2: false,
            basket_realized_broker_only: false,
            closed_profit_net_costs: false,
            restore_strategy_continuation: false,
            msg_kurs_sprzed_luki: false,
            live_tick_order_strict: false,
            // Model poślizgu pendingów jest konfigurowalny per broker.
            slippage_pending_pts: 0.0,
            sim_margin_check_on_fill: true,
            // WYŁĄCZONE dla zgodności z bazą — każdy nowy preset mierzyć z true
            sim_validate_pending_stops: false,
            reenter_respect_cap: false,
            // Z-5: wyłączone dla zgodności z bazą
            sl_edit_reaches_pendings: false,
            // Z-9: wyłączone dla zgodności z bazą
            honor_stop_orders: false,
            // Z-3 / Z-7 / Z-8 / Z-10: wyłączone dla zgodności z bazą
            hint_veto: false,
            // F1: weto odpowiedzi — wyłączone dla zgodności z bazą
            reply_veto: false,
            sync_only_live_levels: true,
            tp_correction_to_broker: false,
            runner_max_hold_rule_only: false,
            runner_max_hold_bez_reguly: false,
            spp_arms_runner_clock: false,
            tp_hit_match_level: false,
            tp_unindexed_pips_require_price: false,
            tp_price_only_strict: false,
            rf_level_sanity_max_usd: 0.0,
            reply_graph_transitive: false,
            stop_out_level_pct: 20.0,
            sim_margin_at_market: false,
            margin_call_level_pct: 50.0,
            server_tz_offset_ms: 3 * 3_600_000,
            msg_clock_offset_ms: None,
            exec_latency_ms: 250,
            slippage_pts: 0.0,

            ai_enabled: false,
            ai_replaces_management: true,
            ai_model: String::new(),
            ai_decision_interval_s: 2.0,

            // Domyślne wartości odtwarzają zachowanie `bot.py` (10 prób co 5 s,
            // potem 1/3/15/30 min…), z jedną poprawką: pierwsza próba nie
            // restartuje działającego terminala.
            mt5_autostart: true,
            mt5_watchdog: true,
            mt5_terminal_path: String::new(),
            mt5_retry_attempts: 10,
            mt5_retry_delay_s: 5.0,
            mt5_restart_after: 1,
            mt5_health_interval_s: 5.0,

            // Dziennik jest DOMYŚLNIE WŁĄCZONY. Powód jest prosty: przy
            // wyłączonym nie da się później odpowiedzieć na pytanie „ile bot
            // zarobił wczoraj i która decyzja kosztowała pieniądze" — a
            // odtworzyć zdarzeń z przeszłości się nie da.
            journal_enabled: true,
            journal_min_level: crate::journal::EventLevel::Info,
            journal_snapshots: true,
            journal_excursions: true,
            journal_text_mirror: true,
            journal_retention_days: 90,
            journal_buffer_cap: 20_000,

            // BRAMKI KAPITAŁOWE — wszystkie progi na 0, czyli rodzina
            // wyłączona. Domyślne ustawienia muszą dawać wynik CO DO CENTA
            // taki jak przed dołożeniem tych pól.
            entry_units_small: 1,
            entry_units_small_mult: 0.0,
            risk_per_basket_pct_small: 0.0,
            risk_per_basket_pct_small_mult: 0.0,
            reenter_max_small: 0,
            reenter_max_small_mult: 0.0,
            max_open_positions_small: 0,
            max_open_positions_small_mult: 0.0,
            max_open_baskets_small: 0,
            max_open_baskets_small_mult: 0.0,
            basket_max_age_min_small: 0.0,
            basket_max_age_min_small_mult: 0.0,
            fast_fill_soft_age_min_small: 0.0,
            fast_fill_soft_age_min_small_mult: 0.0,
            market_entry_step_small: 1.0,
            market_entry_step_small_mult: 0.0,
            sl_min_dist_small: 0.0,
            sl_min_dist_small_mult: 0.0,
            lot_percent_small: 1.0,
            lot_percent_small_mult: 0.0,
            lot_max_z_salda: 0.0,
            trail_atr_mult: 0.0,

            vol_size_mode: VolSizeMode::Off,
            vol_size_target: 0.0,
            vol_size_min_mult: 0.0,
            vol_size_max_mult: 0.0,
            vol_size_percentile_okno: 0,
            vol_size_odsezonuj: false,

            odlicz_kredyt: false,
            credit_balance_separate: false,
            kredyt_reczny: 0.0,

            // Wyłączony i bez progu: przy `false` gałąź odczytu
            // geometrycznego jest martwa, a `parser::parse` (czyli wszystkie
            // dotychczasowe wywołania) woła `OpcjeParsera::default()`, gdzie
            // to pole też jest wyłączone. Parytet z definicji.
            parser_geometryczny: false,
            parser_min_pewnosc: 0.0,

            // EA-CORE: WSZYSTKO ZEROWE. To nie jest ostrożność, tylko warunek
            // bramki akceptacji FALI 0 — przy tych wartościach warstwa nie
            // wchodzi do `EaRdzen::puls` w ogóle (a), a gdyby ktoś włączył sam
            // `ea_enabled`, stan zostaje `Neutral` na zawsze i modulatory
            // są jedynkami (b). Oś, która ruszy którąkolwiek liczbę bramki
            // parytetu przy tych wartościach, jest zepsuta.
            ea_enabled: false,
            ea_tick_s: 0.0,
            ea_state_src: default_ea_state_src(),
            ea_defense_enter: 0.0,
            ea_defense_exit: 0.0,
            ea_offense_enter: 0.0,
            ea_offense_exit: 0.0,
            ea_state_dwell_s: 0.0,
            ea_state_ratchet: default_ea_state_ratchet(),
            ea_state_journal: default_ea_state_journal(),
            ea_dozor_sl: false,
            // RODZINA A — komplet w zerze. Każde z ośmiu pól ma tu wpisane
            // „zachowanie dzisiejsze co do bitu"; test `kontrakt_zera_*`
            // z `tests/ea_rodzina_a.rs` pilnuje, że to prawda, a nie opis.
            ea_lot_z_wolnego_marginesu: 0.0,
            ea_stop_dokladek_przy_stracie: 0.0,
            ea_stop_dokladek_powrot: 0.0,
            ea_redukcja_przy_zageszczeniu: 0.0,
            ea_zageszczenie_podloga: 0.0,
            ea_stan_dnia: default_ea_stan_dnia(),
            ea_stan_dnia_prog_sl: default_ea_stan_dnia_prog_sl(),
            ea_stan_dnia_jednostki_mult: default_ea_stan_dnia_jednostki_mult(),
        }
    }
}

fn default_ea_state_src() -> EaStateSrc {
    EaStateSrc::FloatR
}

fn default_ea_state_ratchet() -> EaRatchet {
    EaRatchet::NieLuzujWKoszyku
}

fn default_ea_state_journal() -> bool {
    true
}

fn default_ea_stan_dnia() -> EaStanDnia {
    EaStanDnia::Off
}

/// „Dzień z 2+ SL" — liczba wprost z zadania rodziny A. Pole jest czytane
/// dopiero przy `ea_stan_dnia != Off`, więc niezerowa wartość domyślna nie
/// narusza kontraktu zera.
fn default_ea_stan_dnia_prog_sl() -> u32 {
    2
}

fn default_ea_stan_dnia_jednostki_mult() -> f64 {
    1.0
}

impl Settings {
    /// Ręczne nadpisanie >0, w przeciwnym razie odczyt konta. OFF odliczania =0.
    #[inline]
    pub fn kredyt_skuteczny_z(&self, kredyt_brokera: f64) -> f64 {
        if !self.odlicz_kredyt { return 0.0; }
        let k = if self.kredyt_reczny.is_finite() && self.kredyt_reczny > 0.0 {
            self.kredyt_reczny
        } else { kredyt_brokera };
        if k.is_finite() && k > 0.0 { k } else { 0.0 }
    }

    /// Kwota do progów Drabinki: surowe Balance w MT5, historycznie B−C.
    /// Niezależna od wyboru Balance/Equity/MinOfBoth do wielkości pozycji.
    #[inline]
    pub fn saldo_wlasne(&self, balance: f64, credit: f64) -> f64 {
        if self.credit_balance_separate { balance }
        else { balance - self.kredyt_skuteczny_z(credit) }
    }

    #[inline]
    pub fn podstawa_lota_z_konta(&self, balance: f64, equity: f64, credit: f64) -> f64 {
        let kredyt = self.kredyt_skuteczny_z(credit);
        if self.credit_balance_separate {
            let own_equity = equity - kredyt;
            match self.lot_base {
                PodstawaLota::Balance => balance,
                PodstawaLota::Equity => own_equity,
                PodstawaLota::MinOfBoth => balance.min(own_equity),
            }.max(0.0)
        } else {
            let kapital = match self.lot_base {
                PodstawaLota::Balance => balance,
                PodstawaLota::Equity => equity,
                PodstawaLota::MinOfBoth => balance.min(equity),
            };
            (kapital - kredyt).max(0.0)
        }
    }

    /// Ile dodać do znacznika wiadomości, żeby wyrazić go w zegarze ticków.
    #[inline]
    pub fn msg_offset(&self) -> i64 {
        self.msg_clock_offset_ms.unwrap_or(self.server_tz_offset_ms)
    }

    /// Offset do liczenia godziny i granicy doby Z ZEGARA TICKÓW.
    ///
    /// Zero, i to nie jest przeoczenie: znaczniki ticków są już zapisane w
    /// czasie serwera brokera (patrz `server_tz_offset_ms`), więc dokładanie
    /// czegokolwiek przesunęłoby dobę handlową o trzy godziny. Metoda istnieje
    /// po to, żeby ta decyzja miała JEDNO miejsce w kodzie.
    #[inline]
    pub fn session_offset(&self) -> i64 {
        0
    }

    /// Konfiguracja dziennika wyprowadzona z ustawień.
    ///
    /// Oba offsety biorą się STĄD, a nie z zegara systemowego: `ts_broker`
    /// w dzienniku ma pokazywać tę samą godzinę co MetaTrader, a rotacja ma
    /// iść po dobie handlowej serwera — dokładnie tej, po której silnik
    /// resetuje statystyki dnia.
    #[inline]
    pub fn journal_config(&self) -> crate::journal::JournalConfig {
        crate::journal::JournalConfig {
            enabled: self.journal_enabled,
            min_level: self.journal_min_level,
            snapshots: self.journal_snapshots,
            excursions: self.journal_excursions,
            server_offset_ms: self.server_tz_offset_ms,
            session_offset_ms: self.session_offset(),
            cap: self.journal_buffer_cap.max(64) as usize,
        }
    }

    /// Efektywna liczba jednostek dla danego typu koszyka.
    #[inline]
    pub fn units_for(&self, is_limit: bool) -> u32 {
        if is_limit && self.entry_units_limit > 0 {
            self.entry_units_limit
        } else {
            self.entry_units
        }
    }

    /// Krok siatki zleceń oczekujących w cenie (0 = jeden poziom na krawędzi).
    ///
    /// PPM ma DWA niezależne zastosowania i poprzedni bot rozróżniał je dwoma
    /// przełącznikami: gęstość siatki limitów i odstęp kolejnych wejść
    /// rynkowych. Zlanie ich w jedno pokrętło zmienia zachowanie presetów,
    /// które włączały tylko jedno z nich.
    #[inline]
    pub fn grid_step(&self) -> f64 {
        if self.ppm_enabled && self.ppm_for_limits && self.ppm > 0.0 {
            1.0 / self.ppm
        } else {
            0.0
        }
    }

    /// Odstęp kolejnych wejść RYNKOWYCH i powtórnych wejść (re-entry).
    ///
    /// Zawsze dodatni: krok zerowy oznaczałby dokładanie pozycji na każdym
    /// ticku, dopóki cena stoi w strefie.
    #[inline]
    pub fn market_step(&self) -> f64 {
        self.market_step_from(self.market_entry_step)
    }

    /// To samo, ale z podanym krokiem bazowym — istnieje dla bramki
    /// kapitałowej (`market_entry_step_small`), która podmienia sam krok
    /// i nie może przy okazji zgubić przeliczenia przez PPM.
    #[inline]
    pub fn market_step_from(&self, base_raw: f64) -> f64 {
        let base = if base_raw > 0.0 { base_raw } else { 1.0 };
        if self.ppm_enabled && self.ppm_for_market && self.ppm > 0.0 {
            base / self.ppm
        } else {
            base
        }
    }

    /// Mnożniki wolumenu dla `n` poziomów siatki, **od najgłębszego do
    /// najpłytszego** — czyli w kolejności, w jakiej silnik trzyma poziomy.
    ///
    /// Zwracany wektor ma zawsze średnią 1: wagi mówią, JAK rozłożyć wolumen
    /// wewnątrz koszyka, a nie ile go dołożyć. Rozmiar bezwzględny zostaje
    /// pod kontrolą lota i limitu ryzyka, więc włączenie wag nie może
    /// niepostrzeżenie podnieść ekspozycji.
    ///
    /// Wagi podaje się od najpłytszego wejścia (najgorszy stosunek zysku do
    /// ryzyka) do najgłębszego (najlepszy), bo tak się o nich myśli:
    /// „1,2,4" znaczy „im głębiej, tym więcej".
    /// Układ drabinki z `entry_uklad`, od krawędzi PŁYTKIEJ do GŁĘBOKIEJ.
    ///
    /// Pusty wynik znaczy „pole wyłączone" — wtedy liczbę szczebli i jednostek
    /// wyznaczają `entry_units` i `units_for_level`, dokładnie jak dotąd.
    pub fn uklad_drabinki(&self) -> Vec<u32> {
        let v: Vec<u32> = self
            .entry_uklad
            .split(',')
            .filter_map(|x| x.trim().parse::<i64>().ok())
            .map(|x| x.clamp(0, 9) as u32)
            .collect();
        if v.iter().sum::<u32>() == 0 {
            return Vec::new();
        }
        v
    }

    pub fn depth_multipliers(&self, n: usize) -> Vec<f64> {
        if n == 0 {
            return Vec::new();
        }
        let w: Vec<f64> = self
            .entry_weights
            .split(',')
            .filter_map(|x| x.trim().parse::<f64>().ok())
            .filter(|x| *x > 0.0)
            .collect();
        if w.len() < 2 {
            return vec![1.0; n];
        }
        if n == 1 {
            // jeden poziom to zawsze najgłębsze możliwe wejście
            return vec![1.0];
        }
        let last = w.len() - 1;
        let mut out: Vec<f64> = Vec::with_capacity(n);
        for i in 0..n {
            // i = 0 to poziom NAJGŁĘBSZY, więc udział głębokości maleje z i
            let depth = (n - 1 - i) as f64 / (n - 1) as f64;
            let idx = (depth * last as f64).round() as usize;
            out.push(w[idx.min(last)]);
        }
        let mean = out.iter().sum::<f64>() / n as f64;
        if mean <= 0.0 {
            return vec![1.0; n];
        }
        for v in out.iter_mut() {
            *v /= mean;
        }
        out
    }

    /// Mnożniki wolumenu liczone z WŁASNEGO R:R każdego szczebla.
    ///
    /// `entry_weights` jest drabinką stałą — trzeba z góry zgadnąć proporcję
    /// („1,2,4"). Tutaj waga bierze się z geometrii tego konkretnego sygnału:
    /// szczebel o R:R 8,0 dostaje szesnaście razy więcej niż szczebel o R:R
    /// 0,50, o ile pozwoli na to sufit `entry_weights_rr_cap`.
    ///
    /// Zwracany wektor ma średnią 1 — tak samo jak `depth_multipliers`. Wagi
    /// mówią, JAK rozłożyć wolumen w koszyku, nie ile go dołożyć; za wielkość
    /// bezwzględną odpowiada lot i `risk_per_basket_pct`.
    ///
    /// Brak SL albo celu = brak wiedzy o jakości szczebli, więc równe wagi.
    /// Wolimy nie zmienić nic, niż zmienić na oślep.
    pub fn rr_multipliers(&self, prices: &[f64], sl: Option<f64>, tp1: Option<f64>) -> Vec<f64> {
        let n = prices.len();
        if n == 0 {
            return Vec::new();
        }
        let (Some(s), Some(t)) = (sl, tp1) else {
            return vec![1.0; n];
        };
        let mut w: Vec<f64> = Vec::with_capacity(n);
        for p in prices {
            let ryzyko = (p - s).abs();
            let nagroda = (t - p).abs();
            // szczebel bez mierzalnego ryzyka albo bez drogi do celu nie ma
            // policzalnej jakości — cała siatka wraca wtedy do równych wag
            if ryzyko <= 1e-9 || nagroda <= 1e-9 {
                return vec![1.0; n];
            }
            w.push(nagroda / ryzyko);
        }
        let pow = if self.entry_weights_rr_power > 0.0 {
            self.entry_weights_rr_power
        } else {
            1.0
        };
        for x in w.iter_mut() {
            *x = x.powf(pow);
            if !x.is_finite() || *x <= 0.0 {
                return vec![1.0; n];
            }
        }
        let min = w.iter().copied().fold(f64::MAX, f64::min);
        if !(min > 0.0) {
            return vec![1.0; n];
        }
        let cap = if self.entry_weights_rr_cap > 0.0 {
            self.entry_weights_rr_cap
        } else {
            f64::INFINITY
        };
        for x in w.iter_mut() {
            *x = (*x / min).min(cap);
        }
        let mean = w.iter().sum::<f64>() / n as f64;
        if !(mean > 0.0) {
            return vec![1.0; n];
        }
        for x in w.iter_mut() {
            *x /= mean;
        }
        w
    }

    /// Mnożnik liczby jednostek dla danej godziny serwera, wg `units_by_hour`.
    ///
    /// Format „7-11:0.5,15-17:2" — te same przedziały co `session_hours`, plus
    /// mnożnik po dwukropku. Pierwsze pasujące pasmo wygrywa; brak dopasowania
    /// znaczy „bez zmian", a nie „zero".
    pub fn hour_units_mult(&self, hour: u32) -> f64 {
        if self.units_by_hour.trim().is_empty() {
            return 1.0;
        }
        for part in self.units_by_hour.split(',') {
            let mut it = part.split(':');
            let zakres = match it.next() {
                Some(x) => x.trim(),
                None => continue,
            };
            let mult = match it.next().and_then(|x| x.trim().parse::<f64>().ok()) {
                Some(m) if m > 0.0 => m,
                _ => continue,
            };
            let mut g = zakres.split('-');
            let a = g.next().and_then(|x| x.trim().parse::<u32>().ok());
            let b = g.next().and_then(|x| x.trim().parse::<u32>().ok());
            match (a, b) {
                (Some(a), Some(b)) if hour >= a && hour < b => return mult,
                (Some(a), None) if hour == a => return mult,
                _ => {}
            }
        }
        1.0
    }

    /// Ile CAŁYCH pozycji zamknąć, gdy harmonogram mówi `pct` % z `n`.
    ///
    /// Wydzielone, bo to jest miejsce błędu „15 % z 3 pozycji = 0".
    #[inline]
    pub fn bank_count(&self, n: usize, pct: f64) -> usize {
        if n == 0 || pct <= 0.0 {
            return 0;
        }
        let raw = n as f64 * pct / 100.0;
        let c = match self.bank_rounding {
            BankRounding::Nearest => raw.round(),
            BankRounding::Up => (raw - 1e-9).ceil(),
            BankRounding::Down => (raw + 1e-9).floor(),
        };
        (c.max(0.0) as usize).min(n)
    }

    /// Czy przy tym zestawie wolumenów wolno zamykać CZĘŚCI pozycji.
    ///
    /// Warunek jest na minimum, nie na średniej: jedna pozycja 0.01 w koszyku
    /// wystarczy, żeby „zamknij 30 % wolumenu" oznaczało dla niej 100 %.
    pub fn partials_allowed(&self, volumes: &[f64]) -> bool {
        self.partial_close
            && !volumes.is_empty()
            && volumes.iter().all(|v| *v >= self.partial_min_lot - 1e-9)
    }

    /// Mnożnik progów dolarowych, gdy skalujemy je z lotem.
    #[inline]
    pub fn usd_scale(&self, lot: f64) -> f64 {
        if self.usd_scale_with_lot {
            (lot / 0.01).max(1.0)
        } else {
            1.0
        }
    }

    pub fn parse_counts(&self) -> Vec<u32> {
        self.official_counts
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect()
    }

    pub fn parse_tiers(raw: &str) -> Vec<(f64, f64)> {
        raw.split(',')
            .filter_map(|c| {
                let mut it = c.split(':');
                let a = it.next()?.trim().parse::<f64>().ok()?;
                let b = it.next()?.trim().parse::<f64>().ok()?;
                Some((a, b))
            })
            .collect()
    }

    /// Pasma toucherów: „offset_pips:units:tp_index"
    pub fn parse_toucher_bands(&self) -> Vec<(f64, u32, usize)> {
        if self.toucher_bands.trim().is_empty() {
            if self.toucher_units > 0 {
                let idx = if self.toucher_tp_one_based {
                    self.toucher_tp_index.max(1) - 1
                } else {
                    self.toucher_tp_index
                };
                return vec![(0.0, self.toucher_units, idx)];
            }
            return Vec::new();
        }
        self.toucher_bands
            .split(',')
            .filter_map(|c| {
                let p: Vec<&str> = c.split(':').collect();
                if p.len() < 2 {
                    return None;
                }
                let off = p[0].trim().parse::<f64>().ok()? * crate::types::PIP;
                let units = p[1].trim().parse::<u32>().ok()?;
                let tp = p
                    .get(2)
                    .and_then(|x| x.trim().parse::<usize>().ok())
                    .unwrap_or(2);
                Some((off, units, tp.saturating_sub(1)))
            })
            .collect()
    }

    pub fn hours_ok(&self, hour: u32) -> bool {
        if !self.session_filter {
            return true;
        }
        for part in self.session_hours.split(',') {
            let mut it = part.split('-');
            let a = it.next().and_then(|x| x.trim().parse::<u32>().ok());
            let b = it.next().and_then(|x| x.trim().parse::<u32>().ok());
            match (a, b) {
                (Some(a), Some(b)) if hour >= a && hour < b => return true,
                (Some(a), None) if hour == a => return true,
                _ => {}
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zaokraglanie_transzy_ma_trzy_rozne_odpowiedzi() {
        let mut c = Settings::default();
        // 15 % z 3 pozycji = 0.45 — to jest ta pułapka
        c.bank_rounding = BankRounding::Nearest;
        assert_eq!(c.bank_count(3, 15.0), 0, "„nearest\" gubi całą transzę TP1");
        c.bank_rounding = BankRounding::Up;
        assert_eq!(c.bank_count(3, 15.0), 1);
        c.bank_rounding = BankRounding::Down;
        assert_eq!(c.bank_count(3, 15.0), 0);
        // wartość dokładna nie może być podnoszona przez „up"
        c.bank_rounding = BankRounding::Up;
        assert_eq!(c.bank_count(4, 50.0), 2);
        // transza nigdy nie przekracza liczby pozycji
        assert_eq!(c.bank_count(3, 300.0), 3);
        assert_eq!(c.bank_count(0, 50.0), 0);
        assert_eq!(c.bank_count(5, 0.0), 0);
    }

    #[test]
    fn wagi_glebokosci_rosna_w_strone_lepszych_wejsc() {
        let mut c = Settings::default();
        c.entry_weights = "1,2,4".into();
        let m = c.depth_multipliers(3);
        // indeks 0 to poziom NAJGŁĘBSZY — musi dostać największą wagę
        assert!(m[0] > m[1] && m[1] > m[2], "{m:?}");
        // proporcje zachowane: 4 : 2 : 1
        assert!((m[0] / m[2] - 4.0).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn wagi_redystrybuuja_wolumen_a_nie_go_powiekszaja() {
        let mut c = Settings::default();
        c.entry_weights = "1,2,4".into();
        for n in [2usize, 3, 5, 8] {
            let m = c.depth_multipliers(n);
            let mean = m.iter().sum::<f64>() / n as f64;
            assert!(
                (mean - 1.0).abs() < 1e-9,
                "średnia mnożników musi wynosić 1, jest {mean} dla n={n}"
            );
        }
    }

    #[test]
    fn brak_wag_zostawia_drabinke_rowna() {
        let c = Settings::default();
        assert_eq!(c.depth_multipliers(4), vec![1.0; 4]);
        // pojedyncza waga to nie rozkład — nie ma czego różnicować
        let mut c2 = Settings::default();
        c2.entry_weights = "3".into();
        assert_eq!(c2.depth_multipliers(4), vec![1.0; 4]);
        // śmieci w polu też nie mogą zmienić wielkości pozycji
        let mut c3 = Settings::default();
        c3.entry_weights = "abc,,-2".into();
        assert_eq!(c3.depth_multipliers(3), vec![1.0; 3]);
    }

    #[test]
    fn wagi_rozciagaja_sie_na_dowolna_liczbe_poziomow() {
        let mut c = Settings::default();
        c.entry_weights = "1,4".into();
        let m = c.depth_multipliers(6);
        assert_eq!(m.len(), 6);
        assert!(m[0] > m[5], "najgłębszy poziom nadal największy: {m:?}");
        // jeden poziom to zawsze najgłębsze możliwe wejście
        assert_eq!(c.depth_multipliers(1), vec![1.0]);
        assert!(c.depth_multipliers(0).is_empty());
    }

    #[test]
    fn partiale_wymagaja_zeby_kazda_pozycja_byla_dosc_duza() {
        let mut c = Settings::default();
        c.partial_close = true;
        c.partial_min_lot = 0.02;
        assert!(c.partials_allowed(&[0.05, 0.02, 0.10]));
        // jedna pozycja poniżej progu wyłącza tryb dla CAŁEGO koszyka:
        // dla niej „30 %" oznaczałoby 100 %
        assert!(!c.partials_allowed(&[0.05, 0.01]));
        assert!(!c.partials_allowed(&[]));
        c.partial_close = false;
        assert!(!c.partials_allowed(&[0.10, 0.10]));
    }

    #[test]
    fn ppm_ma_dwa_niezalezne_zastosowania() {
        let mut c = Settings::default();
        c.ppm_enabled = true;
        c.ppm = 2.0;
        c.market_entry_step = 1.0;

        c.ppm_for_limits = true;
        c.ppm_for_market = false;
        assert_eq!(c.grid_step(), 0.5, "gęstsza siatka limitów");
        assert_eq!(c.market_step(), 1.0, "krok wejść rynkowych bez zmian");

        c.ppm_for_limits = false;
        c.ppm_for_market = true;
        assert_eq!(c.grid_step(), 0.0);
        assert_eq!(c.market_step(), 0.5);
    }

    #[test]
    fn krok_wejsc_rynkowych_nigdy_nie_jest_zerowy() {
        let mut c = Settings::default();
        c.market_entry_step = 0.0;
        assert_eq!(c.market_step(), 1.0, "zero oznaczałoby dokładanie co tick");
    }


    /// Strefa 5 $ szeroka, SL 1 $ pod dolną krawędzią, TP1 3 $ nad górną.
    /// Szczebel dolny ma R:R 8/1, górny 3/6 — szesnastokrotna różnica jakości,
    /// dokładnie ta, którą zmierzyliśmy na kanale.
    #[test]
    fn wagi_z_rr_daja_wiecej_szczeblowi_o_lepszym_stosunku() {
        let mut c = Settings::default();
        c.entry_weights_rr_cap = 0.0; // bez sufitu — chcemy zobaczyć czysty stosunek
        let ceny = vec![4000.0, 4005.0];
        let m = c.rr_multipliers(&ceny, Some(3999.0), Some(4008.0));
        // dolny: (4008−4000)/(4000−3999) = 8 ; górny: (4008−4005)/(4005−3999) = 0.5
        assert!(m[0] > m[1], "{m:?}");
        assert!(
            (m[0] / m[1] - 16.0).abs() < 1e-6,
            "stosunek jakości 16× — {m:?}"
        );
        let mean = (m[0] + m[1]) / 2.0;
        assert!(
            (mean - 1.0).abs() < 1e-9,
            "wagi mają redystrybuować, nie powiększać: {mean}"
        );
    }

    #[test]
    fn sufit_wag_rr_nie_pozwala_zawiesic_koszyka_na_jednym_poziomie() {
        let mut c = Settings::default();
        c.entry_weights_rr_cap = 4.0;
        let ceny = vec![4000.0, 4005.0];
        let m = c.rr_multipliers(&ceny, Some(3999.0), Some(4008.0));
        assert!(
            (m[0] / m[1] - 4.0).abs() < 1e-6,
            "sufit 4× musi obciąć 16× — {m:?}"
        );
        let mean = (m[0] + m[1]) / 2.0;
        assert!((mean - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wykladnik_lagodzi_albo_zaostrza_wagi_rr() {
        let ceny = vec![4000.0, 4005.0];
        let mut c = Settings::default();
        c.entry_weights_rr_cap = 0.0;
        c.entry_weights_rr_power = 0.5;
        let m = c.rr_multipliers(&ceny, Some(3999.0), Some(4008.0));
        assert!(
            (m[0] / m[1] - 4.0).abs() < 1e-6,
            "pierwiastek z 16 to 4 — {m:?}"
        );
    }

    /// Brak wiedzy nie może zmieniać wielkości pozycji. To ta sama zasada,
    /// przez którą `vol_factor` zwraca 1.0 przy zbyt małej liczbie próbek.
    #[test]
    fn brak_sl_albo_celu_zostawia_wagi_rowne() {
        let c = Settings::default();
        let ceny = vec![4000.0, 4005.0];
        assert_eq!(c.rr_multipliers(&ceny, None, Some(4008.0)), vec![1.0, 1.0]);
        assert_eq!(c.rr_multipliers(&ceny, Some(3999.0), None), vec![1.0, 1.0]);
        // szczebel dokładnie na SL: brak mierzalnego ryzyka
        assert_eq!(
            c.rr_multipliers(&ceny, Some(4000.0), Some(4008.0)),
            vec![1.0, 1.0]
        );
        // szczebel dokładnie na celu: brak drogi do zysku
        assert_eq!(
            c.rr_multipliers(&ceny, Some(3990.0), Some(4005.0)),
            vec![1.0, 1.0]
        );
        assert!(c.rr_multipliers(&[], Some(1.0), Some(2.0)).is_empty());
    }


    #[test]
    fn pasma_godzinowe_zmieniaja_liczbe_jednostek() {
        let mut c = Settings::default();
        c.units_by_hour = "7-11:0.5,15-17:2".into();
        assert_eq!(c.hour_units_mult(8), 0.5);
        assert_eq!(c.hour_units_mult(15), 2.0);
        assert_eq!(c.hour_units_mult(16), 2.0);
        // koniec przedziału jest wyłączny — tak samo jak w `hours_ok`
        assert_eq!(c.hour_units_mult(11), 1.0);
        assert_eq!(c.hour_units_mult(17), 1.0);
        // godzina spoza pasm zostaje bez zmian, nie zeruje się
        assert_eq!(c.hour_units_mult(3), 1.0);
    }

    #[test]
    fn puste_i_bledne_pasma_godzinowe_nic_nie_zmieniaja() {
        let c = Settings::default();
        assert_eq!(c.hour_units_mult(12), 1.0);
        let mut c2 = Settings::default();
        c2.units_by_hour = "abc,7-11,,9-10:-3,12-13:0".into();
        for h in 0..24 {
            assert_eq!(
                c2.hour_units_mult(h),
                1.0,
                "godzina {h} nie może zmienić rozmiaru"
            );
        }
    }

    #[test]
    fn ustawienia_przechodza_przez_json_bez_strat() {
        let mut c = Settings::default();
        c.smart_sl_mode = SmartSlMode::LadderWithBe;
        c.bank_from = BankFrom::Best;
        c.last_runner = LastRunner::NoTp;
        c.reenter_after_tp = true;
        let s = serde_json::to_string(&c).unwrap();
        let back: Settings = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn brak_klucza_w_presecie_bierze_domyslna_z_silnika() {
        let c: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            c.sync_only_live_levels,
            Settings::default().sync_only_live_levels,
            "preset bez klucza dostał wartość INNĄ niż domyślna silnika —              sprawdź, czy pole nie ma własnego `#[serde(default)]`"
        );
        assert!(c.sync_only_live_levels, "domyślna od 18.08.2026 to `true`");
        // wartość PODANA w presecie zawsze wygrywa
        let c: Settings = serde_json::from_str(r#"{"sync_only_live_levels": false}"#).unwrap();
        assert!(
            !c.sync_only_live_levels,
            "jawny zapis w presecie musi być uszanowany"
        );
    }

    #[test]
    fn front_run_tp_ma_zero_contract_i_raportuje_ujemna_martwa_wartosc() {
        let c: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(c.tp_price_front_run_usd, 0.0);
        assert!(!c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "tp_price_front_run_usd"));

        let mut c = Settings::default();
        c.tp_price_front_run_usd = -0.2;
        assert!(c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "tp_price_front_run_usd"));

        c.tp_price_front_run_usd = 0.2;
        c.tp_source = TpSource::SignalOnly;
        assert!(c
            .pulapki_konfiguracji()
            .iter()
            .any(|x| x.contains("niezależną drogę CENOWĄ")));
    }

    #[test]
    fn hybryda_market_raportuje_martwe_parametry_i_sprzeczne_bramki() {
        let mut c = Settings::default();
        c.market_hybrid_pending_units = 3;
        c.market_hybrid_lot_mult = 0.5;
        c.market_hybrid_max_chase_usd = 0.7;
        c.market_hybrid_tp_stage = 2;
        let m = c.martwe_ustawienia();
        for pole in [
            "market_hybrid_pending_units",
            "market_hybrid_lot_mult",
            "market_hybrid_max_chase_usd",
            "market_hybrid_tp_stage",
        ] {
            assert!(
                m.iter().any(|x| x.pole == pole),
                "{pole} nie zostało zgłoszone przy wyłączonej hybrydzie: {m:?}"
            );
        }

        c.market_hybrid_now_units = 1;
        let m = c.martwe_ustawienia();
        for pole in [
            "market_hybrid_pending_units",
            "market_hybrid_lot_mult",
            "market_hybrid_max_chase_usd",
            "market_hybrid_tp_stage",
        ] {
            assert!(
                !m.iter().any(|x| x.pole == pole),
                "{pole} pozostaje martwe mimo włączonej hybrydy: {m:?}"
            );
        }

        c.auto_limit = false;
        assert!(c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "market_hybrid_now_units" && x.wlacznik == "auto_limit"));

        c.auto_limit = true;
        c.only_limit_signals = true;
        c.market_unfilled_cancel_stage = 1;
        let m = c.martwe_ustawienia();
        assert!(m.iter().any(|x| {
            x.pole == "market_hybrid_now_units" && x.wlacznik == "only_limit_signals"
        }));
        assert!(m.iter().any(|x| x.pole == "market_unfilled_cancel_stage"));
    }

    #[test]
    fn cala_rodzina_riskfree_ma_jawna_bramke_aktywacji() {
        let mut c = Settings::default();
        c.riskfree_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        c.riskfree_runner_stop = RiskFreeRunnerStop::BeOwn;
        c.pending_cancel_on_riskfree = true;
        let m = c.martwe_ustawienia();
        for pole in [
            "riskfree_runner_target",
            "riskfree_runner_stop",
            "pending_cancel_on_riskfree",
        ] {
            assert!(
                m.iter().any(|x| x.pole == pole),
                "{pole} nie zostało zgłoszone przy riskfree_enabled=false: {m:?}"
            );
        }

        c.riskfree_enabled = true;
        let m = c.martwe_ustawienia();
        for pole in [
            "riskfree_runner_target",
            "riskfree_runner_stop",
            "pending_cancel_on_riskfree",
        ] {
            assert!(
                !m.iter().any(|x| x.pole == pole),
                "{pole} pozostaje martwe mimo riskfree_enabled=true: {m:?}"
            );
        }
    }

    #[test]
    fn bramka_kapitalowa_bez_progu_jest_martwym_ustawieniem() {
        let mut c = Settings::default();
        c.entry_units = 5;
        c.entry_units_small = 3;
        let m = c.martwe_ustawienia();
        assert!(
            m.iter().any(|x| x.pole == "entry_units_small"),
            "brak ostrzeżenia o bramce bez progu: {m:?}"
        );
        // z progiem pole już żyje i ostrzeżenia być nie może
        c.entry_units_small_mult = 2.0;
        assert!(!c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "entry_units_small"));
        // wartość równa bazowej nie jest awarią — nie ma czego zgłaszać
        let mut c2 = Settings::default();
        c2.sl_min_dist_small = c2.sl_min_dist;
        assert!(!c2
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "sl_min_dist_small"));
    }

    #[test]
    fn rodziny_z_audytu_k4_sa_wykrywane() {
        let nowe = [
            "expo_cap_close",
            "expo_cap_s",
            "pending_resize_s",
            "merge_min_overlap",
            "smart_exit_take",
            "trend_filter_drop_pct",
            "vol_size_target",
            "rearm_max_times",
            "slhit_pause_min",
            "regime_percentyl",
            "pyramid_min_equity_mult",
            "fast_addon_lot_mult",
            "exit_round_step",
            "riskfree_keep_units",
            "official_counts",
            "parser_min_pewnosc",
        ];

        // czysty default nie zgłasza ŻADNEJ z nowych rodzin — inaczej każdy
        // wydany preset zacząłby krzyczeć od progu. (Default NIE jest pusty:
        // `session_hours`, punkty swapu i zegar runnera M15 krzyczą z fabryki
        // — to znane, celowe głosy spoza tej listy.)
        let md = Settings::default().martwe_ustawienia();
        for pole in nowe {
            assert!(
                !md.iter().any(|x| x.pole == pole),
                "{pole} krzyczy na czystym default"
            );
        }

        // po jednym reprezentancie z każdej z 15 rodzin
        let mut c = Settings::default();
        c.expo_cap_close = true; //           expo_cap_pct/ml_pct = 0
        c.expo_cap_s = 5.0;
        c.pending_resize_s = 60.0; //         pending_resize_on_vol = false
        c.merge_min_overlap = 0.9; //         merge_same_side = false
        c.smart_exit_take = 12.0; //          smart_exit = false
        c.trend_filter_drop_pct = 3.0; //     trend_filter_enabled = false
        c.vol_size_target = 8.0; //           vol_size_mode = Off
        c.rearm_max_times = 4; //             rearm_grid_on_return = false
        c.slhit_pause_min = 90.0; //          slhit_pause_n = 0
        c.regime_percentyl = 80.0; //         regime_filter = Off
        c.pyramid_min_equity_mult = 1.5; //   pyramid_after_stage = 0
        c.fast_addon_lot_mult = 2.0; //       fast_addon_move_usd = 0
        c.exit_round_step = 25.0; //          exit_round_dist = 0
        c.riskfree_keep_units = 3; //         riskfree_enabled = false
        c.official_counts = "2,2".into(); //  tp_schedule = Ladder
        c.parser_min_pewnosc = 0.6; //        parser_geometryczny = false
        let m = c.martwe_ustawienia();
        for pole in nowe {
            assert!(
                m.iter().any(|x| x.pole == pole),
                "brak zgłoszenia dla {pole}: {m:?}"
            );
        }

        // włączenie włącznika gasi zgłoszenie rodziny — po jednym dowodzie
        // dla bramki bool, enum i progowej
        let mut c2 = c.clone();
        c2.smart_exit = true;
        c2.regime_filter = RegimeFilter::TrendMa;
        c2.fast_addon_move_usd = 3.0;
        let m2 = c2.martwe_ustawienia();
        for pole in ["smart_exit_take", "regime_percentyl", "fast_addon_lot_mult"] {
            assert!(
                !m2.iter().any(|x| x.pole == pole),
                "{pole} zgłoszony mimo włącznika: {m2:?}"
            );
        }

        // `official_pct` żyje TYLKO w OfficialPct; w OfficialCounts jest
        // martwe tak samo jak w Ladder — warunek per pole, nie per rodzina
        let mut c3 = Settings::default();
        c3.tp_schedule = TpSchedule::OfficialCounts;
        c3.official_pct = [40.0, 30.0, 20.0, 10.0];
        assert!(c3
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "official_pct"));
        assert!(!c3
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "official_counts"));
    }

    #[test]
    fn krok_wejsc_rynkowych_z_podmieniona_baza() {
        // `market_step_from` musi zachowywać przeliczenie przez PPM, inaczej
        // bramka kapitałowa po cichu wyłączałaby PPM razem z krokiem.
        let mut c = Settings::default();
        c.market_entry_step = 0.3;
        c.ppm_enabled = true;
        c.ppm_for_market = true;
        c.ppm = 2.0;
        assert_eq!(c.market_step(), 0.15);
        assert_eq!(c.market_step_from(0.6), 0.3);
        // krok zerowy nadal znaczy „jeden dolar", a nie „na każdym ticku"
        assert_eq!(c.market_step_from(0.0), 0.5);
    }

    #[test]
    fn brakujace_klucze_biora_wartosc_domyslna() {
        // `#[serde(default)]` na strukturze: stary plik konfiguracji nie może
        // wywrócić wczytywania po dołożeniu pola
        let c: Settings = serde_json::from_str(r#"{"lot_fixed":0.05}"#).unwrap();
        assert_eq!(c.lot_fixed, 0.05);
        assert_eq!(c.bank_rounding, BankRounding::Up);
        assert_eq!(c.reenter_min_tp_stage, 1);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MartweUstawienie {
    /// pole, które nic nie robi
    pub pole: &'static str,
    /// włącznik, którego brakuje
    pub wlacznik: &'static str,
    /// czytelne wyjaśnienie do dziennika i do panelu
    pub opis: String,
}

impl Settings {
    /// Wykrywa ustawienia o realnej wartości, które silnik zignoruje, bo ich
    /// włącznik jest wyłączony.
    ///
    /// Sygnatura tej awarii jest charakterystyczna i warto ją znać:
    /// **wynik identyczny co do dolara przy różnych wartościach parametru**.
    /// Jeśli przemiatasz oś i wszystkie punkty dają tę samą liczbę, prawie na
    /// pewno patrzysz na zbramkowane pole, a nie na płaski wpływ.
    /// M15: czy `riskfree_runner_max_hold_min` jest wpisany, ale nie ma szans
    /// zadziałać. DWIE drogi do śmierci, nie jedna — druga była przeoczona:
    ///
    ///  * reguła wyłączona i brak `runner_max_hold_bez_reguly` — limit siedzi
    ///    za `if !riskfree_enabled { return; }` w `riskfree_pass`,
    ///  * `runner_max_hold_rule_only` przy wyłączonej regule — `secured_by_rule`
    ///    ustawia WYŁĄCZNIE `riskfree_pass`, więc filtr w
    ///    `limit_trzymania_runnera` zostawia pusty zbiór koszyków. Zegar biegnie
    ///    i nie domyka NICZEGO, a samo `bez_reguly = true` uśpiłoby ostrzeżenie.
    ///
    /// Wspólne dla `martwe_ustawienia` i `pulapki_konfiguracji`, żeby obie
    /// bramki nie rozjechały się przy następnej zmianie.
    fn zegar_runnera_martwy(&self) -> bool {
        self.riskfree_runner_max_hold_min > 0.0
            && !self.riskfree_enabled
            && (!self.runner_max_hold_bez_reguly || self.runner_max_hold_rule_only)
    }

    pub fn martwe_ustawienia(&self) -> Vec<MartweUstawienie> {
        let mut v = Vec::new();
        let mut zglos = |pole: &'static str, wlacznik: &'static str, co: String| {
            v.push(MartweUstawienie {
                pole,
                wlacznik,
                opis: co,
            });
        };

        if self.runner_partial_pct != 0.0 {
            zglos("runner_partial_pct", "brak_implementacji",
                "runner_partial_pct nie ma wykonawczego czytelnika; ta wartość NIE steruje inkasem. Użyj sprawdzonego harmonogramu/partial_close albo pozostaw 0.".into());
        }
        if self.tp_price_only_strict && self.tp_source != TpSource::PriceOnly {
            zglos("tp_price_only_strict", "tp_source=PriceOnly",
                "tp_price_only_strict dotyczy wyłącznie PriceOnly; nie zmienia innych źródeł TP.".into());
        }
        if self.retarget_respects_final_target && !self.cele_na_ostatnim {
            zglos("retarget_respects_final_target", "cele_na_ostatnim",
                "Ta korekta retarget działa tylko przy cele_na_ostatnim; nie zmienia innych harmonogramów.".into());
        }

        // Sygnał rynkowy w trybie hybrydowym ma jedną jawną bramkę:
        // `market_hybrid_now_units > 0`. Bez niej pozostałe pokrętła nie są
        // wariantami strategii, tylko martwymi liczbami. Dodatkowo wykonanie
        // hybrydy celowo wymaga `auto_limit`, bo jego drugą połową są zlecenia
        // oczekujące. Te zależności muszą być widoczne przed sweepem.
        let hybrid_def = Settings::default();
        let hybrid_diff = |a: f64, b: f64| (a - b).abs() > 1e-12;
        if self.market_hybrid_now_units == 0 {
            for (pole, rozna) in [
                (
                    "market_hybrid_pending_units",
                    self.market_hybrid_pending_units != hybrid_def.market_hybrid_pending_units,
                ),
                (
                    "market_hybrid_lot_mult",
                    hybrid_diff(
                        self.market_hybrid_lot_mult,
                        hybrid_def.market_hybrid_lot_mult,
                    ),
                ),
                (
                    "market_hybrid_max_chase_usd",
                    hybrid_diff(
                        self.market_hybrid_max_chase_usd,
                        hybrid_def.market_hybrid_max_chase_usd,
                    ),
                ),
                (
                    "market_hybrid_tp_stage",
                    self.market_hybrid_tp_stage != hybrid_def.market_hybrid_tp_stage,
                ),
            ] {
                if rozna {
                    zglos(
                        pole,
                        "market_hybrid_now_units",
                        format!(
                            "{pole} nie działa — ustaw `market_hybrid_now_units > 0`"
                        ),
                    );
                }
            }
        } else if !self.auto_limit {
            zglos(
                "market_hybrid_now_units",
                "auto_limit",
                "hybryda `teraz + limity` nie działa przy `auto_limit = false` — włącz `auto_limit`"
                    .into(),
            );
        }
        if self.only_limit_signals && self.market_hybrid_now_units > 0 {
            zglos(
                "market_hybrid_now_units",
                "only_limit_signals",
                "hybryda obsługuje sygnały bez `LIMITS`, ale `only_limit_signals = true` odrzuca je przed wykonaniem"
                    .into(),
            );
        }
        if self.only_limit_signals
            && self.market_unfilled_cancel_stage != hybrid_def.market_unfilled_cancel_stage
        {
            zglos(
                "market_unfilled_cancel_stage",
                "only_limit_signals",
                "oś sprząta wyłącznie niewypełnione sygnały bez `LIMITS`, które `only_limit_signals = true` odrzuca wcześniej"
                    .into(),
            );
        }

        // --- rodzina parametrów adaptacyjnych ---
        if !self.adaptive_params {
            for (pole, wart) in [
                ("sl_min_dist_zone_mult", self.sl_min_dist_zone_mult),
                ("sl_min_dist_atr_mult", self.sl_min_dist_atr_mult),
                ("sl_min_dist_floor", self.sl_min_dist_floor),
                ("sl_min_dist_cap", self.sl_min_dist_cap),
            ] {
                if wart > 0.0 {
                    zglos(
                        pole,
                        "adaptive_params",
                        format!("{pole} = {wart} nie działa — włącz `adaptive_params`"),
                    );
                }
            }
            if !self.units_by_hour.trim().is_empty() {
                zglos(
                    "units_by_hour",
                    "adaptive_params",
                    "units_by_hour nie działa — włącz `adaptive_params`".into(),
                );
            }
        }

        // --- filtr tempa wypełnień ---
        if self.fast_fill_reject_s <= 0.0 {
            if self.fast_fill_soft_age_min > 0.0 {
                zglos(
                    "fast_fill_soft_age_min",
                    "fast_fill_reject_s",
                    "tryb miękki filtra tempa nie działa — ustaw próg `fast_fill_reject_s`".into(),
                );
            }
        }

        // --- ponowne wejście po celu ---
        if !self.reenter_after_tp {
            if self.reenter_max > 0 {
                zglos(
                    "reenter_max",
                    "reenter_after_tp",
                    "limit ponownych wejść nie działa — włącz `reenter_after_tp`".into(),
                );
            }
            if self.reenter_min_return_s > 0.0 {
                zglos(
                    "reenter_min_return_s",
                    "reenter_after_tp",
                    "odstęp ponownego wejścia nie działa — włącz `reenter_after_tp`".into(),
                );
            }
        }

        // --- piramida ---
        if self.pyramid_after_stage == 0 && (self.pyramid_lot_mult - 1.0).abs() > f64::EPSILON {
            zglos(
                "pyramid_lot_mult",
                "pyramid_after_stage",
                "mnożnik piramidy nie działa — ustaw `pyramid_after_stage`".into(),
            );
        }

        // --- BRAMKI KAPITAŁOWE ---
        //
        // Rodzina `*_small` ma dokładnie tę sygnaturę awarii, przed którą
        // ostrzega dokumentacja tej funkcji: wartość wpisana, próg zerowy,
        // wynik identyczny co do dolara przy każdej wartości pola.
        {
            let d = |a: f64, b: f64| (a - b).abs() > 1e-12;
            for (pole, wlacznik, rozna) in [
                (
                    "entry_units_small",
                    "entry_units_small_mult",
                    self.entry_units_small_mult <= 0.0
                        && self.entry_units_small != self.entry_units,
                ),
                (
                    "risk_per_basket_pct_small",
                    "risk_per_basket_pct_small_mult",
                    self.risk_per_basket_pct_small_mult <= 0.0
                        && d(self.risk_per_basket_pct_small, self.risk_per_basket_pct),
                ),
                (
                    "reenter_max_small",
                    "reenter_max_small_mult",
                    self.reenter_max_small_mult <= 0.0
                        && self.reenter_max_small != self.reenter_max,
                ),
                (
                    "max_open_positions_small",
                    "max_open_positions_small_mult",
                    self.max_open_positions_small_mult <= 0.0
                        && self.max_open_positions_small != self.max_open_positions,
                ),
                (
                    "max_open_baskets_small",
                    "max_open_baskets_small_mult",
                    self.max_open_baskets_small_mult <= 0.0
                        && self.max_open_baskets_small != self.max_open_baskets,
                ),
                (
                    "basket_max_age_min_small",
                    "basket_max_age_min_small_mult",
                    self.basket_max_age_min_small_mult <= 0.0
                        && d(self.basket_max_age_min_small, self.basket_max_age_min),
                ),
                (
                    "fast_fill_soft_age_min_small",
                    "fast_fill_soft_age_min_small_mult",
                    self.fast_fill_soft_age_min_small_mult <= 0.0
                        && d(
                            self.fast_fill_soft_age_min_small,
                            self.fast_fill_soft_age_min,
                        ),
                ),
                (
                    "market_entry_step_small",
                    "market_entry_step_small_mult",
                    self.market_entry_step_small_mult <= 0.0
                        && d(self.market_entry_step_small, self.market_entry_step),
                ),
                (
                    "sl_min_dist_small",
                    "sl_min_dist_small_mult",
                    self.sl_min_dist_small_mult <= 0.0
                        && d(self.sl_min_dist_small, self.sl_min_dist),
                ),
                (
                    "lot_percent_small",
                    "lot_percent_small_mult",
                    self.lot_percent_small_mult <= 0.0
                        && d(self.lot_percent_small, self.lot_percent),
                ),
            ] {
                if rozna {
                    zglos(
                        pole,
                        wlacznik,
                        format!(
                            "{pole} nie działa — próg `{wlacznik}` wynosi 0, \
                             więc bramka kapitałowa jest wyłączona"
                        ),
                    );
                }
            }
        }

        // --- filtr sesji ---
        if !self.session_filter && !self.session_hours.trim().is_empty() {
            zglos(
                "session_hours",
                "session_filter",
                "godziny sesji nie działają — włącz `session_filter`".into(),
            );
        }

        // --- swap ---
        if !self.swap_enabled && (self.swap_long_points != 0.0 || self.swap_short_points != 0.0) {
            zglos(
                "swap_long_points / swap_short_points",
                "swap_enabled",
                "punkty swapu nie są naliczane — włącz `swap_enabled`".into(),
            );
        }

        // --- M15: limit trzymania runnera wpisany, ale zbramkowany regułą ---
        //
        // FS-M3-SYN, NEWALPHA-2 i OMEGA-X2 niosą `riskfree_runner_max_hold_min
        // = 90,0`, a wszystkie mają `riskfree_enabled = false`. Limit siedzi
        // w `riskfree_pass`, które zaczyna się od `if !riskfree_enabled
        // { return; }` — więc pole jest widoczne w panelu i BEZ SKUTKU.
        if self.zegar_runnera_martwy() {
            zglos(
                "riskfree_runner_max_hold_min",
                if self.runner_max_hold_bez_reguly {
                    "runner_max_hold_rule_only"
                } else {
                    "runner_max_hold_bez_reguly"
                },
                if self.runner_max_hold_bez_reguly {
                    format!(
                        "riskfree_runner_max_hold_min = {} min nie ma czego domykać — \
                         `runner_max_hold_rule_only` zawęża limit do koszyków uwolnionych \
                         REGUŁĄ (`secured_by_rule`), a przy `riskfree_enabled = false` \
                         reguła nie uwalnia ani jednego koszyka. Wyłącz `rule_only` \
                         albo włącz regułę.",
                        self.riskfree_runner_max_hold_min
                    )
                } else {
                    format!(
                        "riskfree_runner_max_hold_min = {} min nie działa — limit siedzi \
                         w regule `riskfree_enabled`, która jest wyłączona. Włącz \
                         `runner_max_hold_bez_reguly`, żeby dotyczył koszyków uwolnionych \
                         KOMUNIKATEM kanału.",
                        self.riskfree_runner_max_hold_min
                    )
                },
            );
        }

        {
            let def = Settings::default();
            let d = |a: f64, b: f64| (a - b).abs() > 1e-12;

            // ekspozycja potencjalna: przy obu progach zerowych
            // `redukuj_ekspozycje` wychodzi PIERWSZĄ linijką (engine.rs:8238),
            // więc ani tryb domykania, ani kadencja nie są w ogóle czytane
            if self.expo_cap_pct <= 0.0 && self.expo_cap_ml_pct <= 0.0 {
                if self.expo_cap_close {
                    zglos(
                        "expo_cap_close",
                        "expo_cap_pct",
                        "domykanie pozycji pod próg nie działa — oba progi \
                         (`expo_cap_pct` i `expo_cap_ml_pct`) są zerowe"
                            .into(),
                    );
                }
                if d(self.expo_cap_s, def.expo_cap_s) {
                    zglos(
                        "expo_cap_s",
                        "expo_cap_pct",
                        "kadencja pułapu ekspozycji nie działa — oba progi \
                         (`expo_cap_pct` i `expo_cap_ml_pct`) są zerowe"
                            .into(),
                    );
                }
            }

            // przeliczanie lotów w leżących zleceniach (engine.rs:7710)
            if !self.pending_resize_on_vol && d(self.pending_resize_s, def.pending_resize_s) {
                zglos(
                    "pending_resize_s",
                    "pending_resize_on_vol",
                    "kadencja przeliczania lotów w zleceniach nie działa — \
                     włącz `pending_resize_on_vol`"
                        .into(),
                );
            }

            // scalanie koszyków tego samego kierunku (engine.rs:3300)
            if !self.merge_same_side {
                for (pole, rozna) in [
                    (
                        "merge_window_min",
                        d(self.merge_window_min, def.merge_window_min),
                    ),
                    (
                        "merge_min_overlap",
                        d(self.merge_min_overlap, def.merge_min_overlap),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "merge_same_side",
                            format!("{pole} nie działa — włącz `merge_same_side`"),
                        );
                    }
                }
            }

            // inteligentne wyjście (engine.rs:7349)
            if !self.smart_exit {
                for (pole, rozna) in [
                    (
                        "smart_exit_take",
                        d(self.smart_exit_take, def.smart_exit_take),
                    ),
                    (
                        "smart_exit_giveback",
                        d(self.smart_exit_giveback, def.smart_exit_giveback),
                    ),
                    (
                        "smart_exit_min_peak",
                        d(self.smart_exit_min_peak, def.smart_exit_min_peak),
                    ),
                    (
                        "smart_exit_drop_speed",
                        d(self.smart_exit_drop_speed, def.smart_exit_drop_speed),
                    ),
                    (
                        "smart_exit_speed_window_s",
                        d(
                            self.smart_exit_speed_window_s,
                            def.smart_exit_speed_window_s,
                        ),
                    ),
                    (
                        "smart_exit_hold_if_pending",
                        d(
                            self.smart_exit_hold_if_pending,
                            def.smart_exit_hold_if_pending,
                        ),
                    ),
                    (
                        "smart_exit_min_pendings",
                        self.smart_exit_min_pendings != def.smart_exit_min_pendings,
                    ),
                    (
                        "smart_exit_pending_scope",
                        self.smart_exit_pending_scope != def.smart_exit_pending_scope,
                    ),
                    (
                        "smart_exit_pending_min_dist",
                        d(
                            self.smart_exit_pending_min_dist,
                            def.smart_exit_pending_min_dist,
                        ),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "smart_exit",
                            format!("{pole} nie działa — włącz `smart_exit`"),
                        );
                    }
                }
            }

            if !self.trail_sr_enabled {
                for (pole, rozna) in [
                    ("trail_sr_scope", self.trail_sr_scope != def.trail_sr_scope),
                    (
                        "trail_sr_activation",
                        self.trail_sr_activation != def.trail_sr_activation,
                    ),
                    (
                        "trail_sr_min_gain",
                        d(self.trail_sr_min_gain, def.trail_sr_min_gain),
                    ),
                    (
                        "trail_sr_min_dist_price",
                        d(self.trail_sr_min_dist_price, def.trail_sr_min_dist_price),
                    ),
                    (
                        "trail_sr_min_prominence_atr",
                        d(
                            self.trail_sr_min_prominence_atr,
                            def.trail_sr_min_prominence_atr,
                        ),
                    ),
                    (
                        "trail_sr_offset_atr_mult",
                        d(self.trail_sr_offset_atr_mult, def.trail_sr_offset_atr_mult),
                    ),
                    (
                        "trail_sr_offset_spread_mult",
                        d(
                            self.trail_sr_offset_spread_mult,
                            def.trail_sr_offset_spread_mult,
                        ),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "trail_sr_enabled",
                            format!("{pole} nie działa — włącz `trail_sr_enabled`"),
                        );
                    }
                }
            } else if self.trail_sr_activation != TrailSrActivation::Gain
                && d(self.trail_sr_min_gain, def.trail_sr_min_gain)
            {
                zglos(
                    "trail_sr_min_gain",
                    "trail_sr_activation",
                    "próg zysku nie działa — czytany wyłącznie przy \
                     `trail_sr_activation = Gain`"
                        .into(),
                );
            }
            let sr_dynamiczny = self.trail_sr_min_prominence_atr > 0.0
                || self.trail_sr_offset_atr_mult > 0.0
                || self.trail_sr_offset_spread_mult > 0.0;
            if !sr_dynamiczny && self.trail_sr_atr_period != def.trail_sr_atr_period {
                zglos(
                    "trail_sr_atr_period",
                    "trail_sr_min_prominence_atr|trail_sr_offset_atr_mult|trail_sr_offset_spread_mult",
                    "okres ATR jest czytany dopiero po włączeniu co najmniej jednej \
                     dynamicznej osi S/R"
                        .into(),
                );
            }

            // filtr trendu (engine.rs:9452 — wymaga OBU: włącznika i progu)
            if !self.trend_filter_enabled {
                for (pole, rozna) in [
                    (
                        "trend_filter_window_h",
                        d(self.trend_filter_window_h, def.trend_filter_window_h),
                    ),
                    (
                        "trend_filter_drop_pct",
                        d(self.trend_filter_drop_pct, def.trend_filter_drop_pct),
                    ),
                    (
                        "trend_filter_mode",
                        self.trend_filter_mode != def.trend_filter_mode,
                    ),
                    (
                        "trend_filter_shrink",
                        d(self.trend_filter_shrink, def.trend_filter_shrink),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "trend_filter_enabled",
                            format!("{pole} nie działa — włącz `trend_filter_enabled`"),
                        );
                    }
                }
            }

            // rozmiar sterowany zmiennością (engine.rs:856 — Off nie wchodzi
            // w gałąź; do tego Target nie czyta okna percentyla i odwrotnie)
            if self.vol_size_mode == VolSizeMode::Off {
                if d(self.vol_size_target, def.vol_size_target) {
                    zglos(
                        "vol_size_target",
                        "vol_size_mode",
                        "docelowy zasięg nie działa — ustaw `vol_size_mode = Target`".into(),
                    );
                }
                if self.vol_size_percentile_okno != def.vol_size_percentile_okno {
                    zglos(
                        "vol_size_percentile_okno",
                        "vol_size_mode",
                        "okno percentyla nie działa — ustaw `vol_size_mode = Percentile`".into(),
                    );
                }
            }

            // ponowne uzbrojenie siatki po powrocie ceny (engine.rs:9618)
            if !self.rearm_grid_on_return {
                for (pole, rozna) in [
                    ("rearm_keep_empty_alive", self.rearm_keep_empty_alive),
                    (
                        "spp_blocks_rearm_when_flat",
                        self.spp_blocks_rearm_when_flat,
                    ),
                    (
                        "rearm_min_basket_profit",
                        d(self.rearm_min_basket_profit, def.rearm_min_basket_profit),
                    ),
                    (
                        "rearm_max_times",
                        self.rearm_max_times != def.rearm_max_times,
                    ),
                    (
                        "rearm_min_gap_min",
                        d(self.rearm_min_gap_min, def.rearm_min_gap_min),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "rearm_grid_on_return",
                            format!("{pole} nie działa — włącz `rearm_grid_on_return`"),
                        );
                    }
                }
            }

            // pauza po stopach (engine.rs:2245 — próg `n` bramkuje resztę;
            // UWAGA na konwencję zera bliźniaków: przy WŁĄCZONEJ regule
            // `slhit_pause_min = 0` to pauza DO KOŃCA DOBY, podczas gdy
            // `streak_pause_min = 0` wyłącza pauzę — tu zgłaszamy tylko
            // martwość przy `n = 0`, semantyk nie ruszamy)
            if self.slhit_pause_n == 0 {
                for (pole, rozna) in [
                    (
                        "slhit_pause_min",
                        d(self.slhit_pause_min, def.slhit_pause_min),
                    ),
                    (
                        "slhit_pause_lot_mult",
                        d(self.slhit_pause_lot_mult, def.slhit_pause_lot_mult),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "slhit_pause_n",
                            format!("{pole} nie działa — ustaw próg `slhit_pause_n`"),
                        );
                    }
                }
            }

            // filtr reżimu (engine.rs:9556/10429 — Off gasi CAŁĄ rodzinę,
            // łącznie z trybem miękkim)
            if self.regime_filter == RegimeFilter::Off {
                for (pole, rozna) in [
                    (
                        "regime_ma_hours",
                        d(self.regime_ma_hours, def.regime_ma_hours),
                    ),
                    ("regime_cena", self.regime_cena != def.regime_cena),
                    ("regime_miara", self.regime_miara != def.regime_miara),
                    ("regime_pilnuj_limitow", self.regime_pilnuj_limitow),
                    (
                        "regime_percentyl",
                        d(self.regime_percentyl, def.regime_percentyl),
                    ),
                    (
                        "regime_strefa_martwa",
                        d(self.regime_strefa_martwa, def.regime_strefa_martwa),
                    ),
                    ("regime_okno2_h", d(self.regime_okno2_h, def.regime_okno2_h)),
                    (
                        "regime_zmiennosc_min",
                        d(self.regime_zmiennosc_min, def.regime_zmiennosc_min),
                    ),
                    (
                        "regime_zmiennosc_max",
                        d(self.regime_zmiennosc_max, def.regime_zmiennosc_max),
                    ),
                    (
                        "regime_gdy_rozerwany",
                        self.regime_gdy_rozerwany != def.regime_gdy_rozerwany,
                    ),
                    ("regime_soft", self.regime_soft),
                    (
                        "regime_soft_units_mult",
                        d(self.regime_soft_units_mult, def.regime_soft_units_mult),
                    ),
                    (
                        "regime_soft_lot_mult",
                        d(self.regime_soft_lot_mult, def.regime_soft_lot_mult),
                    ),
                    (
                        "regime_soft_max_positions",
                        self.regime_soft_max_positions != def.regime_soft_max_positions,
                    ),
                    (
                        "regime_soft_risk_mult",
                        d(self.regime_soft_risk_mult, def.regime_soft_risk_mult),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "regime_filter",
                            format!("{pole} nie działa — ustaw `regime_filter` inny niż Off"),
                        );
                    }
                }
            }

            // piramida (engine.rs:5057 — `after_stage = 0` gasi wszystkie
            // warunki dokładek, także reżimowy i equity)
            if self.pyramid_after_stage == 0 {
                for (pole, rozna) in [
                    (
                        "pyramid_regime_lookback",
                        self.pyramid_regime_lookback != def.pyramid_regime_lookback,
                    ),
                    (
                        "pyramid_regime_max_fast_pct",
                        d(
                            self.pyramid_regime_max_fast_pct,
                            def.pyramid_regime_max_fast_pct,
                        ),
                    ),
                    (
                        "pyramid_min_equity_mult",
                        d(self.pyramid_min_equity_mult, def.pyramid_min_equity_mult),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "pyramid_after_stage",
                            format!("{pole} nie działa — ustaw `pyramid_after_stage`"),
                        );
                    }
                }
            }

            // szybka dokładka (engine.rs:9219 — `move_usd <= 0` = reguła
            // wyłączona; w produkcji 0 wszędzie, więc reszta rodziny to atrapy)
            if self.fast_addon_move_usd <= 0.0 {
                for (pole, rozna) in [
                    (
                        "fast_addon_window_s",
                        d(self.fast_addon_window_s, def.fast_addon_window_s),
                    ),
                    ("fast_addon_max", self.fast_addon_max != def.fast_addon_max),
                    (
                        "fast_addon_lot_mult",
                        d(self.fast_addon_lot_mult, def.fast_addon_lot_mult),
                    ),
                    (
                        "fast_addon_min_stage",
                        self.fast_addon_min_stage != def.fast_addon_min_stage,
                    ),
                    (
                        "fast_addon_cooldown_s",
                        d(self.fast_addon_cooldown_s, def.fast_addon_cooldown_s),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "fast_addon_move_usd",
                            format!("{pole} nie działa — ustaw próg `fast_addon_move_usd`"),
                        );
                    }
                }
            }

            // wyjście przy okrągłym poziomie (engine.rs:7319 — wymaga OBU pól)
            if self.exit_round_dist <= 0.0 && d(self.exit_round_step, def.exit_round_step) {
                zglos(
                    "exit_round_step",
                    "exit_round_dist",
                    "krok okrągłych poziomów nie działa — ustaw `exit_round_dist`".into(),
                );
            }

            // reguła risk-free (engine.rs `riskfree_pass` zaczyna się od
            // `if !riskfree_enabled { return; }` — cała rodzina za jedną
            // bramką; `riskfree_runner_max_hold_min` obsłużony wyżej w M15,
            // bo ma DRUGĄ drogę życia przez `runner_max_hold_bez_reguly`)
            if !self.riskfree_enabled {
                for (pole, rozna) in [
                    (
                        "riskfree_trigger_usd",
                        d(self.riskfree_trigger_usd, def.riskfree_trigger_usd),
                    ),
                    (
                        "riskfree_trigger_r",
                        d(self.riskfree_trigger_r, def.riskfree_trigger_r),
                    ),
                    (
                        "riskfree_keep_units",
                        self.riskfree_keep_units != def.riskfree_keep_units,
                    ),
                    (
                        "riskfree_be_offset",
                        d(self.riskfree_be_offset, def.riskfree_be_offset),
                    ),
                    (
                        "riskfree_runner_gap",
                        d(self.riskfree_runner_gap, def.riskfree_runner_gap),
                    ),
                    (
                        "riskfree_runner_target",
                        self.riskfree_runner_target != def.riskfree_runner_target,
                    ),
                    (
                        "riskfree_runner_stop",
                        self.riskfree_runner_stop != def.riskfree_runner_stop,
                    ),
                    (
                        "pending_cancel_on_riskfree",
                        self.pending_cancel_on_riskfree != def.pending_cancel_on_riskfree,
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "riskfree_enabled",
                            format!("{pole} nie działa — włącz `riskfree_enabled`"),
                        );
                    }
                }
            }

            // Ujemna wartość autonomicznego front-runu jest celowo
            // normalizowana do zera w `Engine::on_tick`, więc nie może
            // zmienić ani jednego etapu. UI nie pozwala jej wpisać, ale
            // ręcznie edytowany preset ma dostać jawny raport martwej osi.
            if self.tp_price_front_run_usd < 0.0 {
                zglos(
                    "tp_price_front_run_usd",
                    "tp_price_front_run_usd > 0",
                    "ujemny front-run jest normalizowany do 0 i nie działa — ustaw wartość > 0"
                        .into(),
                );
            }

            // harmonogramy oficjalne (engine.rs:5446/5455 — `official_pct`
            // czyta wyłącznie tryb OfficialPct, `official_counts` wyłącznie
            // OfficialCounts; każde pole ma więc WŁASNY warunek trybu)
            if self.tp_schedule != TpSchedule::OfficialPct && self.official_pct != def.official_pct
            {
                zglos(
                    "official_pct",
                    "tp_schedule",
                    "harmonogram procentowy nie działa — ustaw `tp_schedule = OfficialPct`".into(),
                );
            }
            if self.tp_schedule != TpSchedule::OfficialCounts
                && self.official_counts != def.official_counts
            {
                zglos(
                    "official_counts",
                    "tp_schedule",
                    "harmonogram liczbowy nie działa — ustaw `tp_schedule = OfficialCounts`".into(),
                );
            }

            // próg jakości odczytu geometrycznego (parser.rs — przy wyłączonym
            // przełączniku pole nie jest w ogóle czytane; a przy WŁĄCZONYM
            // zero znaczy „bierz każdy odczyt", nie „wyłączone")
            if !self.parser_geometryczny && self.parser_min_pewnosc > 0.0 {
                zglos(
                    "parser_min_pewnosc",
                    "parser_geometryczny",
                    "próg pewności odczytu nie działa — włącz `parser_geometryczny`".into(),
                );
            }

            // EA-CORE: `ea_enabled = false` gasi CAŁĄ warstwę strukturalnie
            // (silnik nie wchodzi do `EaRdzen::puls` w ogóle), więc każde
            // ustawione pole rodziny jest martwe co do bitu — a nie „prawie
            // martwe". To jest ta sama sytuacja co `trail_sr_enabled` obok.
            if !self.ea_enabled {
                for (pole, rozna) in [
                    ("ea_tick_s", d(self.ea_tick_s, def.ea_tick_s)),
                    ("ea_state_src", self.ea_state_src != def.ea_state_src),
                    (
                        "ea_defense_enter",
                        d(self.ea_defense_enter, def.ea_defense_enter),
                    ),
                    (
                        "ea_defense_exit",
                        d(self.ea_defense_exit, def.ea_defense_exit),
                    ),
                    (
                        "ea_offense_enter",
                        d(self.ea_offense_enter, def.ea_offense_enter),
                    ),
                    (
                        "ea_offense_exit",
                        d(self.ea_offense_exit, def.ea_offense_exit),
                    ),
                    (
                        "ea_state_dwell_s",
                        d(self.ea_state_dwell_s, def.ea_state_dwell_s),
                    ),
                    (
                        "ea_state_ratchet",
                        self.ea_state_ratchet != def.ea_state_ratchet,
                    ),
                    (
                        "ea_state_journal",
                        self.ea_state_journal != def.ea_state_journal,
                    ),
                    ("ea_dozor_sl", self.ea_dozor_sl != def.ea_dozor_sl),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "ea_enabled",
                            format!("{pole} nie działa — włącz `ea_enabled` (warstwa EA)"),
                        );
                    }
                }
            }

            // Wewnątrz WŁĄCZONEJ warstwy zero nadal znaczy „nigdy": próg
            // wyjścia bez progu wejścia to pole, którego nikt nie przeczyta,
            // bo do stanu nie da się wejść.
            if self.ea_enabled {
                if self.ea_defense_enter <= 0.0 && self.ea_defense_exit > 0.0 {
                    zglos(
                        "ea_defense_exit",
                        "ea_defense_enter",
                        "próg wyjścia z obrony nie działa — `ea_defense_enter = 0` \
                         znaczy OBRONA NIGDY, więc nie ma z czego wychodzić"
                            .into(),
                    );
                }
                if self.ea_offense_enter <= 0.0 && self.ea_offense_exit > 0.0 {
                    zglos(
                        "ea_offense_exit",
                        "ea_offense_enter",
                        "próg wyjścia z agresji nie działa — `ea_offense_enter = 0` \
                         znaczy AGRESJA NIGDY, więc nie ma z czego wychodzić"
                            .into(),
                    );
                }
                if self.ea_defense_enter <= 0.0
                    && self.ea_offense_enter <= 0.0
                    && self.ea_state_dwell_s > 0.0
                {
                    zglos(
                        "ea_state_dwell_s",
                        "ea_defense_enter",
                        "minimalny czas trwania stanu nie ma czego pilnować — oba progi \
                         wejścia (`ea_defense_enter`, `ea_offense_enter`) są zerowe, \
                         więc stan zostaje `Neutral` na zawsze"
                            .into(),
                    );
                }
            }

            // ---- RODZINA A ----
            //
            // Brama rodziny to `ea_enabled` **albo** tryb AUTO-EA, a tryb jest
            // własnością SILNIKA (`Engine::tryb_auto_ea`), nie presetu — więc
            // `Settings` nie ma jak sprawdzić, czy pole na pewno jest martwe.
            // Ostrzeżenie mówi to WPROST zamiast udawać pewność, której nie ma.
            if !self.ea_enabled {
                for (pole, rozna) in [
                    (
                        "ea_lot_z_wolnego_marginesu",
                        d(
                            self.ea_lot_z_wolnego_marginesu,
                            def.ea_lot_z_wolnego_marginesu,
                        ),
                    ),
                    (
                        "ea_stop_dokladek_przy_stracie",
                        d(
                            self.ea_stop_dokladek_przy_stracie,
                            def.ea_stop_dokladek_przy_stracie,
                        ),
                    ),
                    (
                        "ea_stop_dokladek_powrot",
                        d(self.ea_stop_dokladek_powrot, def.ea_stop_dokladek_powrot),
                    ),
                    (
                        "ea_redukcja_przy_zageszczeniu",
                        d(
                            self.ea_redukcja_przy_zageszczeniu,
                            def.ea_redukcja_przy_zageszczeniu,
                        ),
                    ),
                    ("ea_stan_dnia", self.ea_stan_dnia != def.ea_stan_dnia),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "ea_enabled",
                            format!(
                                "{pole} (rodzina A) działa tylko przy `ea_enabled` \
                                 ALBO w trybie AUTO-EA — w zwykłym AUTO/MANUAL/AI \
                                 jest martwe co do bitu"
                            ),
                        );
                    }
                }
            }

            // Pola drugiego rzędu rodziny A: bez swojej osi nadrzędnej nikt
            // ich nie czyta — dokładnie tak samo, jak `ea_defense_exit` bez
            // `ea_defense_enter` wyżej.
            if self.ea_stop_dokladek_przy_stracie <= 0.0 && self.ea_stop_dokladek_powrot > 0.0 {
                zglos(
                    "ea_stop_dokladek_powrot",
                    "ea_stop_dokladek_przy_stracie",
                    "próg powrotu nie działa — `ea_stop_dokladek_przy_stracie = 0` \
                     znaczy STOP DOKŁADKOM NIGDY, więc nie ma czego zdejmować"
                        .into(),
                );
            }
            if self.ea_stop_dokladek_przy_stracie > 0.0
                && self.ea_stop_dokladek_powrot > self.ea_stop_dokladek_przy_stracie
            {
                zglos(
                    "ea_stop_dokladek_powrot",
                    "ea_stop_dokladek_przy_stracie",
                    "histereza odwrócona — próg powrotu musi być MNIEJ dotkliwy niż \
                     próg wejścia, inaczej weto zdejmuje się w tej samej chwili, \
                     w której się zatrzasnęło"
                        .into(),
                );
            }
            if self.ea_redukcja_przy_zageszczeniu <= 0.0 && self.ea_zageszczenie_podloga > 0.0 {
                zglos(
                    "ea_zageszczenie_podloga",
                    "ea_redukcja_przy_zageszczeniu",
                    "podłoga mnożnika nie działa — nachylenie zagęszczenia jest zerowe, \
                     więc mnożnik zostaje 1,0 na zawsze"
                        .into(),
                );
            }
            if matches!(self.ea_stan_dnia, EaStanDnia::Off)
                && (self.ea_stan_dnia_prog_sl != def.ea_stan_dnia_prog_sl
                    || d(
                        self.ea_stan_dnia_jednostki_mult,
                        def.ea_stan_dnia_jednostki_mult,
                    ))
            {
                zglos(
                    "ea_stan_dnia_prog_sl",
                    "ea_stan_dnia",
                    "próg i mnożnik stanu dnia nie działają — `ea_stan_dnia = Off`".into(),
                );
            }
            if self.ea_stan_dnia_jednostki_mult > 1.0 {
                zglos(
                    "ea_stan_dnia_jednostki_mult",
                    "ea_stan_dnia",
                    "mnożnik > 1 jest PRZYCINANY do 1,0 — „ryzyko nie rośnie po stracie\" \
                     jest niezmiennikiem osi A4, nie ustawieniem"
                        .into(),
                );
            }
        }

        v
    }

    /// Ostrzeżenia, które NIE są zbramkowaniem, ale zaskakują tak samo mocno.
    ///
    /// Różnica wobec `martwe_ustawienia`: tam pole nie robi NIC. Tu robi coś
    /// innego, niż sugeruje nazwa albo sąsiednie ustawienie.
    pub fn pulapki_konfiguracji(&self) -> Vec<String> {
        let mut v = Vec::new();

        if self.pending_relot_reconcile_target {
            v.push("pending_relot_reconcile_target: pełny checked plan zastępuje pending_relot_wg_planu; sync/rearm rewalidują legalny wolumen obu trybów. RequiresReview blokuje nowe wejścia danego poziomu, nie jest kolejką zleceń po restarcie.".into());
            if !self.pending_relot_on_balance {
                v.push("pending_relot_reconcile_target: cykliczny relot jest wyłączony przez pending_relot_on_balance=false; kontrola bezpiecznego sync/rearm i zapisane RequiresReview nadal obowiązują.".into());
            }
        }

        if self.entry_edit_geometry_v2 {
            v.push("entry_edit_geometry_v2: wymaga źródłowego snapshotu. Kosmetyczna edycja nie zmienia zleceń. Working/partial geometry i niepotwierdzony cancel/fill przechodzą w RequiresReview tylko danego koszyka; obecny live nie potwierdza automatycznej wymiany geometrii. Zapisany review nie jest kolejką replay ani gwarancją atomic restart.".into());
        }

        if self.order_volume_contract_v2 {
            // Static preflight has no broker/account snapshot. Dynamic bounds
            // and the actual broker lattice are checked at every opening.
            if !self.lot_min.is_finite() || self.lot_min <= 0.0
                || !self.lot_max.is_finite() || self.lot_max < 0.0
                || !self.lot_max_z_salda.is_finite() || self.lot_max_z_salda < 0.0
                || (self.lot_max > 0.0 && self.lot_max < self.lot_min) {
                v.push("order_volume_contract_v2: niepoprawne min/max lub dzielnik kapitału; nowe wejścia zostaną odrzucone (fail-closed), bez zamiany granic i bez podnoszenia resztkowego wolumenu.".into());
            }
            v.push("order_volume_contract_v2 wymaga znanego dodatniego min/step/max brokera i kroku reprezentowalnego do 8 miejsc. lot_max=0 wyłącza tylko limit użytkownika, nie limit brokera. Wolumen jest zaokrąglany w dół; zlecenie poniżej minimum nie powstanie.".into());
        }

        if self.tp_source == TpSource::PriceOnly && self.tp_unindexed_pips_require_price
            && !self.tp_price_only_strict {
            v.push("PriceOnly ma aktywny wyjątek legacy: +N PIPS HIT może awansować etap po kontroli ceny. tp_price_only_strict=true usuwa ten wyjątek, nie wyłączając RF/SPP/SL.".into());
        }

        if self.tp_price_front_run_usd > 0.0 && matches!(self.tp_source, TpSource::SignalOnly) {
            v.push(format!(
                "`tp_source = SignalOnly`, ale `tp_price_front_run_usd = {}` celowo \
                 dodaje niezależną drogę CENOWĄ dla koszyków z pozycją. \
                 Telegram nie jest potrzebny; zero przywraca czyste SignalOnly.",
                self.tp_price_front_run_usd
            ));
        }
        if self.tp_price_front_run_usd > 0.0 && self.assign_tp_per_position {
            v.push(format!(
                "`tp_price_front_run_usd = {}` wykonuje zarządzanie etapem przed TP, \
                 ale `assign_tp_per_position = true` pozostawia brokerowe TP na pełnym \
                 poziomie. Gdy zlecenie front-run zostanie odrzucone, broker zamknie \
                 pozycję dopiero na jej zwykłym TP; dziennik musi pokazać wynik close.",
                self.tp_price_front_run_usd
            ));
        }

        // Najdroższa pułapka projektu: 1054 $ na jednym polu.
        if matches!(self.trail_mode, TrailMode::Off)
            && !matches!(self.trail_runner_mode, TrailMode::Off)
            && (self.trail_split || (self.risk_free_trail && !self.riskfree_enabled))
        {
            v.push(format!(
                "`trail_mode = Off` NIE wyłącza trailingu runnerów — \
                 `trail_runner_mode = {:?}` może działać przez trail_split lub kanałowy risk_free_trail; próg wynosi {} $ zysku. \
                 Zmierzony koszt tej niespodzianki: 1054 $.",
                self.trail_runner_mode, self.trail_runner_start
            ));
        }

        {
            let bez_celu = matches!(
                self.risk_free_runner_target,
                RiskFreeRunnerTarget::NoTpTrailOnly
            );
            let bez_trailingu = matches!(self.trail_mode, TrailMode::Off)
                && matches!(self.trail_runner_mode, TrailMode::Off);
            // `be_offset` większy od typowego dystansu do TP1 znaczy, że
            // `sl_is_valid` odrzuci BE, zanim cena zdąży do niego dojść.
            let be_poza_zasiegiem = self.be_offset >= 9.0;
            let bez_zegara =
                self.riskfree_runner_max_hold_min <= 0.0 || self.zegar_runnera_martwy();
            if bez_celu && bez_trailingu && be_poza_zasiegiem && bez_zegara {
                v.push(format!(
                    "RUNNER NIE MA WYJŚCIA: `risk_free_runner_target = NoTpTrailOnly` \
                     zdejmuje cel i deleguje pilnowanie do trailingu, \
                     `trail_mode`/`trail_runner_mode = Off` trailing wyłączają, \
                     `be_offset = {}` sprawia, że `sl_is_valid` odrzuca stop na BE, \
                     a limit `riskfree_runner_max_hold_min` nie obowiązuje. \
                     Po RISK FREE pozycja nie ma ANI celu, ANI zapadki, ANI terminu. \
                     Najtańsze wyjście: `runner_max_hold_bez_reguly = true` \
                     (nie stawia sufitu i nie dotyka stopu).",
                    self.be_offset
                ));
            }
        }

        // Podłoga equity jest stanem pochłaniającym — konto zamiera i nie wraca.
        if self.equity_floor_pct > 0.0 {
            v.push(format!(
                "`equity_floor_pct = {}` blokuje NOWE wejścia poniżej progu i jest \
                 STANEM POCHŁANIAJĄCYM: konto zamiera i nie wznowi się samo, \
                 wymaga ręcznej decyzji. W pracy bez nadzoru oznacza trwałe zatrzymanie wejść.",
                self.equity_floor_pct
            ));
        }

        if self.max_dd_pct > 0.0 || self.max_dd_usd > 0.0 {
            v.push(format!(
                "`max_dd_pct = {}` / `max_dd_usd = {}` ZAMYKA WSZYSTKO po cenie dna \
                 i wstrzymuje wejścia (także dokładki i odbudowę siatki). \
                 Przed użyciem oceń wpływ progu na ogon zysków i ryzyko zatrzymania; \
                 `max_portfolio_risk_pct` ogranicza ekspozycję bez trwałego haltu.",
                self.max_dd_pct, self.max_dd_usd
            ));
        }

        // Zasięg `LifetimePeakDailyReset` obiecuje zwolnienie, którego nie daje.
        if (self.max_dd_pct > 0.0 || self.max_dd_usd > 0.0)
            && matches!(self.dd_guard_scope, DdGuardScope::LifetimePeakDailyReset)
        {
            v.push(
                "`dd_guard_scope = LifetimePeakDailyReset` mierzy obsunięcie od szczytu \
                 WSZECH CZASÓW, a blokadę zdejmuje o północy. Jeśli equity trwale spadło \
                 poniżej progu, strażnik zapala się PONOWNIE na pierwszym ticku każdego \
                 dnia — zwolnienie jest pozorne, blokada dożywotnia."
                    .to_string(),
            );
        }

        // Dławik bez limitu koszykowego nie ma czego mnożyć.
        if (self.dd_soft_pct > 0.0 || self.dd_hard_pct > 0.0) && self.risk_per_basket_pct <= 0.0 {
            v.push(
                "`dd_soft_pct`/`dd_hard_pct` mnożą BUDŻET RYZYKA KOSZYKA, a \
                 `risk_per_basket_pct = 0` znaczy, że tego budżetu nie ma. \
                 Dławik jest wtedy martwy — ustaw `risk_per_basket_pct` albo użyj \
                 `max_portfolio_risk_pct`."
                    .to_string(),
            );
        }

        // Limit pozycji sprawdzany tylko przy sygnale — obietnica bez pokrycia.
        if self.max_open_positions > 0 && !self.enforce_position_limit_on_fill {
            v.push(format!(
                "`max_open_positions = {}` jest sprawdzany TYLKO w chwili nadejścia \
                 sygnału. Wypełnienia wiszących limitów już mu nie podlegają, więc \
                 jeden koszyk potrafi mieć więcej pozycji niż limit. \
                 Włącz `enforce_position_limit_on_fill`, jeśli limit ma naprawdę obowiązywać.",
                self.max_open_positions
            ));
        }

        if self.ea_enabled && self.ea_tick_s <= 0.0 {
            v.push(
                "`ea_tick_s = 0` NIE znaczy \"zegar wyłączony\" — znaczy \
                 \"BEZ WŁASNEGO ZEGARA, czyli puls na KAŻDYM tiku\". To jest \
                 najbardziej kosztowny tryb: każda zmiana ceny uruchamia \
                 dodatkową pracę i może wymagać odczytu rachunku z terminala."
                    .to_string(),
            );
        }

        v
    }
}

pub fn nieznane_pola_ustawien(surowy: &serde_json::Value) -> Vec<String> {
    let Some(fields) = surowy.get("settings").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let known = settings_deserialization_field_names();
    let mut unknown: Vec<String> = fields.keys()
        .filter(|key| !known.contains(key.as_str())).cloned().collect();
    unknown.sort();
    unknown
}

// Derive accepted names (including aliases) from the actual serde visitor.
// Removing an alias set to its default does not change Settings, but that does
// NOT mean serde ignored it. Invalid values elsewhere broke that heuristic too.
// This function classifies names only; normal serde still validates values.
fn settings_deserialization_field_names() -> &'static std::collections::HashSet<&'static str> {
    use serde::de::{self, Visitor};
    static KNOWN: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    KNOWN.get_or_init(|| {
        struct Names<'a>(&'a mut std::collections::HashSet<&'static str>);
        impl<'de> de::Deserializer<'de> for Names<'_> {
            type Error = de::value::Error;
            fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
                Err(de::Error::custom("field-name discovery has no input values"))
            }
            fn deserialize_struct<V: Visitor<'de>>(self, _: &'static str,
                fields: &'static [&'static str], _: V) -> Result<V::Value, Self::Error> {
                self.0.extend(fields.iter().copied());
                Err(de::Error::custom("field names captured; no Settings value requested"))
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map enum identifier ignored_any
            }
        }
        let mut names = std::collections::HashSet::new();
        let _ = Settings::deserialize(Names(&mut names));
        assert!(!names.is_empty(), "Settings no longer exposes serde struct field names");
        names
    })
}

// Historical implementation retained only for a real RED/control regression.
#[cfg(test)]
fn legacy_unknown_settings_by_value(surowy: &serde_json::Value) -> Vec<String> {
    let Some(obiekt) = surowy.get("settings").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let wzorzec = match serde_json::to_value(Settings::default()) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return Vec::new(),
    };
    let podejrzani: Vec<String> = obiekt
        .keys()
        .filter(|k| !wzorzec.contains_key(*k))
        .cloned()
        .collect();
    if podejrzani.is_empty() {
        return Vec::new();
    }

    //  ROZSTRZYGNIECIE BEZ LISTY ALIASOW: usuwamy klucz i patrzymy, czy
    //  wynik parsowania sie ZMIENIL. Jesli nie — serde go nie czytal, czyli
    //  jest naprawde pomijany. Jesli tak — byl aliasem albo nazwa kanoniczna
    //  pod inna postacia. Ta proba nie wymaga zadnej listy do utrzymania,
    //  a wiec nie ma jak sie zdezaktualizowac.
    let pelny: Settings =
        serde_json::from_value(serde_json::Value::Object(obiekt.clone())).unwrap_or_default();
    let mut out = Vec::new();
    for k in podejrzani {
        let mut bez = obiekt.clone();
        bez.remove(&k);
        let okrojony: Settings =
            serde_json::from_value(serde_json::Value::Object(bez)).unwrap_or_default();
        if okrojony == pelny {
            out.push(k);
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod recognized_settings_names_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn genuine_default_alias_reproduces_legacy_false_alarm_but_is_now_known() {
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        for (alias, canonical) in [("regime_range_mute_usd", "regime_zmiennosc_max"),
                                  ("regime_range_mute_mode", "regime_gdy_rozerwany")] {
            let doc = json!({"settings": {alias: defaults[canonical].clone()}});
            assert!(serde_json::from_value::<Settings>(doc["settings"].clone()).is_ok());
            assert_eq!(legacy_unknown_settings_by_value(&doc), vec![alias.to_string()],
                "historical heuristic really misclassified this accepted default alias");
            assert!(nieznane_pola_ustawien(&doc).is_empty());
        }
        assert!(nieznane_pola_ustawien(&json!({"settings": defaults})).is_empty(),
            "all canonical serialized names must remain recognized");
    }

    #[test]
    fn name_classification_is_independent_of_bad_values_elsewhere() {
        let doc = json!({"settings": {"lot_fixed": {"invalid": "type"},
            "regime_range_mute_usd": 0.0,
            "regime_range_mute_mode": {"invalid": "enum"},
            "misspelled_runner_axis": 0, "future_unknown_axis": true}});
        assert!(serde_json::from_value::<Settings>(doc["settings"].clone()).is_err());
        assert_eq!(nieznane_pola_ustawien(&doc), vec!["future_unknown_axis", "misspelled_runner_axis"]);
    }

    #[test]
    fn recognized_aliases_do_not_make_ambiguous_duplicates_valid() {
        let doc = json!({"settings": {"regime_range_mute_usd": 0.0,
                                      "regime_zmiennosc_max": 0.0}});
        assert!(nieznane_pola_ustawien(&doc).is_empty());
        assert!(serde_json::from_value::<Settings>(doc["settings"].clone()).is_err(),
            "canonical plus alias is still an ambiguous duplicate field");
        assert_eq!(nieznane_pola_ustawien(&json!({"settings": {"truely_unknown": 0}})),
            vec!["truely_unknown"]);
    }
}

/// Nazwany zestaw ustawień.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "domyslny_format")]
    pub format: String,
    pub settings: Settings,
    /// KONFIGURACJA WARSTWY EA dla tego presetu — surowy JSON, nietknięty.
    ///
    /// # Po co, skoro jest `CONDUIT_EA_BETA`
    ///
    /// Bo zmienna środowiskowa jest czytana RAZ na proces (`OnceLock`), a więc
    /// jeden przebieg backtestu = jedna konfiguracja EA. Przy takim wiązaniu
    /// przemiat tysiąca konfiguracji EA wymagałby tysiąca uruchomień, z których
    /// KAŻDE wczytuje 116 mln tików od nowa. To nie jest niedogodność, tylko
    /// ściana: EA nie da się wtedy szukać tak, jak szukamy presetów.
    ///
    /// Pole nietknięte (`serde_json::Value`), bo `KonfigBety` mieszka w `core`
    /// obok silnika i ma własną walidację z krzykiem na `stderr`. Przepisywanie
    /// jej tutaj dałoby drugie miejsce, w którym „co jest poprawną konfiguracją
    /// EA" — a dwa takie miejsca zawsze się rozjeżdżają.
    ///
    /// Brak pola = zachowanie dotychczasowe co do bitu: konfigurację bierze
    /// zmienna środowiskowa, a gdy i jej nie ma, warstwa jest wyłączona.
    #[serde(default)]
    pub ea: Option<serde_json::Value>,
}

fn domyslny_format() -> String {
    "ATFX".to_string()
}

/// Domyślna wartość `true` dla pól, których brak w pliku ma znaczyć
/// „włączone". Kontener `Settings` ma `#[serde(default)]`, więc brakujące pole
/// bierze wartość z `Default`, ale pojedyncze `#[serde(default)]` na polu
/// `bool` dałoby `false` — a dla kierunków relotu to zmiana zachowania.
fn prawda() -> bool {
    true
}

/// Domyślna JEDYNKA dla mnożników miękkiego reżimu.
///
/// Musi być funkcją, a nie `#[serde(default)]`, bo `f64::default()` to ZERO —
/// a zerowy mnożnik lota znaczy „nie handluj wcale". Preset zapisany starszą
/// wersją bota nie ma tych pól i wczytałby się jako konfiguracja, która
/// wygląda na miękki reżim, a jest twardą blokadą z dodatkowym krokiem.
fn jeden_f64() -> f64 {
    1.0
}

fn trzy_u32() -> u32 {
    3
}

/// Domyślna cena odniesienia filtra reżimu — rynkowa, czyli parytet.
fn regime_cena_domyslna() -> RegimeCena {
    RegimeCena::Rynkowa
}

/// Domyślna miara progu filtra reżimu — średnia, czyli parytet.
fn regime_miara_domyslna() -> RegimeMiara {
    RegimeMiara::Srednia
}

/// Domyślny percentyl filtra reżimu — mediana.
fn regime_percentyl_domyslny() -> f64 {
    50.0
}

/// Domyślna reakcja na rozerwany rynek — milczenie, czyli parytet.
fn regime_gdy_rozerwany_domyslny() -> RegimeGdyRozerwany {
    RegimeGdyRozerwany::Milcz
}

/// CO filtruje okno godzin handlu.
///
/// Filtr sesji patrzył dotąd wyłącznie na godzinę PRZYJŚCIA sygnału, a
/// wypełnienie zlecenia oczekującego przechodziło o dowolnej porze. Dla
/// strategii, która żyje z limitów, to jest pytanie o pieniądze: sygnał
/// z 20:00 wypełnia się o 9:00, a sygnał z 10:00 potrafi wypełnić się o 6:00.
///
/// Dla zleceń RYNKOWYCH wszystkie warianty znaczą to samo — przyjęcie
/// i wypełnienie dzieją się w tej samej chwili.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SesjaBramka {
    Sygnal,
    /// godzina WYPEŁNIENIA — sygnał wchodzi o każdej porze, ale limity śpią
    /// poza oknem i budzą się, gdy okno się otworzy
    Wypelnienie,
    /// obie godziny muszą się mieścić w oknie
    Oba,
}

/// Domyślna bramka sesji — po godzinie przyjęcia, czyli parytet.
fn sesja_bramka_domyslna() -> SesjaBramka {
    SesjaBramka::Sygnal
}

// ---------- Pakiet A: domyślne wartości osi dedupu i edycji ----------
//
// Funkcje zamiast literałów w dwóch miejscach: `serde(default = …)` i
// `Default for Settings` czytają TĘ SAMĄ wartość, więc nie mogą się rozjechać.

/// A2: pełny status wykonania akcji — czysta poprawka błędu, domyślnie WŁĄCZONA.
/// Domyślnie włączona poprawka księgowania statusu wykonania.
fn default_dedup_pelny_status() -> bool {
    true
}
/// A3: edycja z wejściem wykonuje resztę akcji — domyślnie wyłączone (parytet).
fn default_edycja_wykonuje_reszte_akcji() -> bool {
    false
}
/// A4: klucz dedupu z wartością — domyślnie wyłączone (parytet).
fn default_dedup_klucz_z_wartoscia() -> bool {
    false
}
/// A5: edycja-sierota nie otwiera koszyka — domyślnie wyłączone (parytet).
fn default_edycja_sieroty_nie_otwiera() -> bool {
    false
}
/// A6: idempotencja nowej wiadomości — domyślnie włączona.
fn default_entry_idempotencja() -> bool {
    true
}

// ---------- Pakiet B: domyślne wartości osi z audytu TYLER ----------
//
// Ten sam wzorzec co Pakiet A wyżej: funkcje zamiast literałów, żeby
// `serde(default = …)` i `Default for Settings` czytały TĘ SAMĄ wartość.
// Wszystkie domyślne = kontrakt parytetu (stare zachowanie co do bitu).

/// B1: strażnik intencji dla RISK FREE — domyślnie wyłączony (parytet).
fn default_rf_wymaga_wykonania() -> bool {
    false
}
/// B2: rozmiar wejścia rynkowego — 0 = pełna siatka jak dotąd (parytet).
fn default_market_entry_units() -> u32 {
    0
}
/// B2H: hybryda `teraz + limity` — 0 = stara, jednolita trasa wykonania.
fn default_market_hybrid_now_units() -> u32 {
    0
}
/// B2H: 0 = wszystkie pozostałe pendingi (brak dodatkowego limitu).
fn default_market_hybrid_pending_units() -> u32 {
    0
}
/// B2H: wolumen nogi natychmiastowej bez zmiany.
fn default_market_hybrid_lot_mult() -> f64 {
    1.0
}
/// B2H: bez dodatkowego limitu gonienia strefy.
fn default_market_hybrid_max_chase_usd() -> f64 {
    0.0
}
/// B2H: zachowaj cel wyznaczony przez planer.
fn default_market_hybrid_tp_stage() -> u8 {
    0
}
/// B2C: wcześniejsze sprzątanie market-pendingów — 0 = parytet.
fn default_market_unfilled_cancel_stage() -> u8 {
    0
}
/// B3: kasowanie pendingów przy RISK FREE — domyślnie wyłączone (parytet).
fn default_pending_cancel_on_riskfree() -> bool {
    false
}
/// B4: bank całości na etapie N — 0 = wyłączone (parytet).
fn default_deferred_entry_max_age_s() -> f64 { 300.0 }
fn default_bank_all_at_stage() -> u8 {
    0
}
/// E1: próg BE w raporcie — 0 = tylko dokładne zero (parytet starych tabel).
fn default_stat_be_prog_usd() -> f64 {
    0.0
}

// ---------- Pakiet G: rozdzielone osie kraty, auto-RF, ekspozycja, sesja ----------
//
// Ten sam wzorzec co Pakiety A i B: funkcja zamiast literału, żeby
// `serde(default = …)` i `Default for Settings` czytały TĘ SAMĄ wartość.

/// G1: mnożenie zleceń na poziomie kraty — domyślnie WŁĄCZONE, bo to jest
/// zachowanie sprzed rozdzielenia pól (parytet co do centa).
fn default_units_per_level() -> bool {
    true
}

/// U-BUG38R: kontrakt zera — stare zachowanie (mnożnik kraty łapie strefę).
fn default_units_per_level_zone() -> bool {
    true
}


/// OS_SR: kontrakt zera — rodzina wyłączona, silnik bajt w bajt jak przed nią.
fn default_trail_sr_enabled() -> bool {
    false
}

fn default_trail_sr_scope() -> TrailSrScope {
    TrailSrScope::Runner
}

/// OS_SR: środek plaskowyżu tp2–tp3 najsilniejszej osi (spec pkt 2).
fn default_trail_sr_activation() -> TrailSrActivation {
    TrailSrActivation::Tp2
}

fn default_trail_sr_min_gain() -> f64 {
    3.0
}

/// OS_SR: oddech od ceny — 6 $, koniec plaskowyżu drugiej żywej osi.
fn default_trail_sr_min_dist_price() -> f64 {
    6.0
}


fn default_close_all_scope() -> CloseAllScope {
    CloseAllScope::Global
}

/// Co zrobić ze strefą sygnału PRZECIWNEGO wobec otwartych pozycji.
///
/// Kanał, wysyłając „SELL LIMITS @ 4400/4405" przy naszych otwartych pozycjach
/// długich, mówi wprost, gdzie widzi sufit. Zamknięcie po rynku (`exit_on_
/// opposite_signal`) wyrzuca nas natychmiast i zmierzyliśmy, że kosztuje 55 %.
/// Ustawienie tam CELU nie kosztuje nic, dopóki cena nie dojdzie — a jeśli
/// dojdzie, inkasujemy dokładnie tam, gdzie nadawca spodziewa się odwrócenia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CelZPrzeciwnego {
    Off,
    /// cel na BLIŻSZEJ krawędzi strefy przeciwnej (pierwsza cena nadawcy)
    BliższaKrawedz,
    /// cel na DALSZEJ krawędzi (liczy na przebicie całej strefy nadawcy)
    DalszaKrawedz,
    /// cel na ŚRODKU strefy przeciwnej
    Srodek,
}

/// Domyślnie wyłączone — kontrakt zera.
fn cel_z_przeciwnego_domyslny() -> CelZPrzeciwnego {
    CelZPrzeciwnego::Off
}

#[cfg(test)]
mod credit_balance_separate_tests {
    use super::*;
    use crate::engine::Engine;
    fn cfg() -> Settings { Settings {credit_balance_separate:true,odlicz_kredyt:true,..Settings::default()} }
    #[test]
    fn credit_balance_separate_pu_actual_snapshot_and_quantized_lot() {
        let mut c=cfg();c.lot_mode_percent=true;c.lot_percent=0.5;c.lot_min=0.01;c.lot_max=0.0;
        let mut e=Engine::new(c,159.8);e.stats.equity=459.8;e.stats.credit=300.0;
        assert_eq!(e.podstawa_lota(),159.8);assert_eq!(e.kredyt_odliczony_od_podstawy(),0.0);
        assert_eq!(e.lot_size(e.podstawa_lota()),0.01,"minimum lot still quantizes this small account to .01");
        e.cfg.credit_balance_separate=false;
        assert_eq!(e.podstawa_lota(),0.0);assert_eq!(e.lot_size(e.podstawa_lota()),0.01);
    }
    #[test]
    fn credit_balance_separate_all_capital_selectors_and_floating_loss() {
        let mut c=cfg();
        for (b,e,credit,expected) in [(600.0,900.0,300.0,[600.0,600.0,600.0]),(600.0,800.0,300.0,[600.0,500.0,500.0]),(600.0,950.0,300.0,[600.0,650.0,600.0]),(159.8,459.8,300.0,[159.8,159.8,159.8])] {
            for (base,want) in [PodstawaLota::Balance,PodstawaLota::Equity,PodstawaLota::MinOfBoth].into_iter().zip(expected) {
                c.lot_base=base;assert!((c.podstawa_lota_z_konta(b,e,credit)-want).abs()<1e-9);
            }
        }
    }
    #[test]
    fn credit_balance_separate_off_matches_legacy_formula() {
        let mut c=cfg();c.credit_balance_separate=false;
        for deduct in [false,true] {c.odlicz_kredyt=deduct;
            for manual in [0.0,300.0,500.0] {c.kredyt_reczny=manual;
                for base in [PodstawaLota::Balance,PodstawaLota::Equity,PodstawaLota::MinOfBoth] {
                    c.lot_base=base;let raw: f64=match base{PodstawaLota::Balance=>600.0,PodstawaLota::Equity=>450.0,PodstawaLota::MinOfBoth=>450.0};
                    let credit: f64=if !deduct {0.0}else if manual>0.0{manual}else{300.0};
                    assert_eq!(c.podstawa_lota_z_konta(600.0,450.0,300.0).to_bits(),(raw-credit).max(0.0).to_bits());
                }
            }
        }
    }
    #[test]
    fn credit_balance_separate_zero_credit_bitwise_parity() {
        let mut c=cfg();
        for base in [PodstawaLota::Balance,PodstawaLota::Equity,PodstawaLota::MinOfBoth] {
            c.lot_base=base;
            for (b,e) in [(0.0,0.0),(600.0,500.0),(600.0,700.0),(-50.0,-50.0)] {
                c.credit_balance_separate=true;let on=c.podstawa_lota_z_konta(b,e,0.0);
                c.credit_balance_separate=false;assert_eq!(on.to_bits(),c.podstawa_lota_z_konta(b,e,0.0).to_bits());
            }
        }
    }
    #[test]
    fn credit_balance_separate_bonus_removal_manual_override_and_reload() {
        let mut c=cfg();c.lot_base=PodstawaLota::Equity;
        assert_eq!(c.podstawa_lota_z_konta(600.0,900.0,300.0),600.0);
        assert_eq!(c.podstawa_lota_z_konta(600.0,600.0,0.0),600.0);
        c.kredyt_reczny=250.0;assert_eq!(c.podstawa_lota_z_konta(600.0,900.0,300.0),650.0);
        c.odlicz_kredyt=false;assert_eq!(c.podstawa_lota_z_konta(600.0,900.0,300.0),900.0);
        c.odlicz_kredyt=true;c.lot_base=PodstawaLota::Balance;assert_eq!(c.podstawa_lota_z_konta(600.0,900.0,300.0),600.0);
        let mut e=Engine::new(c,600.0);e.stats.equity=900.0;e.stats.credit=300.0;
        assert_eq!(e.podstawa_lota(),600.0);e.cfg.credit_balance_separate=false;assert_eq!(e.podstawa_lota(),350.0);
    }
    #[test]
    fn credit_balance_separate_actual_deduction_min_is_not_nominal_credit() {
        let mut e=Engine::new(cfg(),600.0);e.stats.equity=850.0;e.stats.credit=300.0;
        e.cfg.lot_base=PodstawaLota::MinOfBoth;assert_eq!(e.kredyt_odliczony_od_podstawy(),50.0);
        e.cfg.lot_base=PodstawaLota::Equity;assert_eq!(e.kredyt_odliczony_od_podstawy(),300.0);
        e.cfg.lot_base=PodstawaLota::Balance;assert_eq!(e.kredyt_odliczony_od_podstawy(),0.0);
        assert_eq!(e.cfg.saldo_wlasne(600.0,300.0),600.0);
    }
    #[test]
    fn credit_balance_separate_serde_default_and_account_mapping() {
        assert!(!Settings::default().credit_balance_separate);
        let missing:Settings=serde_json::from_str("{}").unwrap();assert!(!missing.credit_balance_separate);
        let yes:Settings=serde_json::from_str("{\"credit_balance_separate\":true}").unwrap();assert!(yes.credit_balance_separate);
        assert!(crate::wielosilnik::POLA_RACHUNKU.contains(&"credit_balance_separate"));
        let merged=crate::wielosilnik::ustawienia_formatu(&Settings::default(),&yes);assert!(merged.credit_balance_separate);
    }
}
