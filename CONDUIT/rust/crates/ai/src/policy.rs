//! Polityka: dwie małe sieci MLP + dekodowanie akcji + serializacja modelu.
//!
//! **Dlaczego nie `burn` / nie autograd.** Nie potrzebujemy gradientów — model
//! jest trenowany ewolucyjnie (patrz `train.rs`), a jedyna operacja w środku
//! pętli to mnożenie macierzy 60×48. Zwykłe `Vec<f32>` daje: kilkanaście sekund
//! kompilacji zamiast kilkunastu minut, pełny determinizm (kolejność sumowania
//! jest ustalona), zerowe alokacje na inferencję i trywialne zrównoleglenie
//! rolloutów przez `rayon`.
//!
//! **Dwie sieci, nie jedna.** Decyzje mają dwa różne poziomy:
//!  * `bsk` — co zrobić z KOSZYKIEM (limity, dołożenie wejścia, zamknięcie),
//!  * `pos` — co zrobić z KONKRETNĄ pozycją (wyjście, SL, TP).
//!
//! Rozdzielenie ich sprawia, że liczba wyjść nie zależy od liczby pozycji —
//! ta sama sieć obsługuje koszyk z jedną i z dwudziestoma pozycjami.

use crate::obs::{BSK_IN, POS_IN};
use crate::safety::SafetyCfg;
use rand::Rng;
use serde::{Deserialize, Serialize};

// ============================================================
//  WYJŚCIA
// ============================================================

/// Wyjścia sieci pozycji.
pub const O_HOLD: usize = 0;
pub const O_CLOSE: usize = 1;
pub const O_PARTIAL: usize = 2;
pub const O_PART_FRAC: usize = 3;
pub const O_SL_KEEP: usize = 4;
pub const O_SL_SET: usize = 5;
pub const O_SL_GAP: usize = 6;
pub const O_TP_KEEP: usize = 7;
pub const O_TP_SET: usize = 8;
pub const O_TP_MULT: usize = 9;
pub const O_TP_DROP: usize = 10;
pub const POS_OUT: usize = 11;

/// Wyjścia sieci koszyka.
pub const B_HOLD: usize = 0;
pub const B_CANCEL: usize = 1;
pub const B_ADD: usize = 2;
pub const B_CLOSE: usize = 3;
pub const B_ADD_DIST: usize = 4;
pub const B_ADD_LOT: usize = 5;
pub const B_ADD_TP: usize = 6;
pub const B_REPOS: usize = 7;
pub const B_DEPTH: usize = 8;
pub const B_SL: usize = 9;
pub const B_UNITS: usize = 10;
pub const B_PAUSE: usize = 11;
pub const B_PAUSE_H: usize = 12;
pub const BSK_OUT: usize = 13;

/// Ile osi decyzyjnych dostaje model.
///
/// **Dlaczego to jest przełącznik, a nie stała.** Pełna przestrzeń akcji
/// (wyjście, część, SL, TP, limity, dokładki, przestawienie) okazała się
/// nieuczalna na tych danych: rozrzut ocen kandydatów w populacji wynosi ±0.05,
/// a cały sygnał najlepszego presetu to +0.022. Szum estymaty gradientu jest
/// dwukrotnie większy niż to, czego model ma się nauczyć, więc ewolucja
/// schodziła do bezczynności. `EntryOnly` zostawia DWIE osie — głębokość wejścia
/// i wielkość pozycji — czyli dokładnie te, które mają uzasadnienie ekonomiczne
/// (R:R 0.5 przy krawędzi płytkiej wobec 8.0 przy głębokiej). Reszta zarządzania
/// wraca do silnika.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionMode {
    /// wszystkie akcje: zarządzanie pozycją + koszykiem + wejściem
    Full,
    /// tylko głębokość wejścia i wielkość pozycji; SL/TP/wyjścia robi silnik
    EntryOnly,
}

impl Default for ActionMode {
    fn default() -> Self {
        ActionMode::EntryOnly
    }
}

impl ActionMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "full" | "pelny" => Some(ActionMode::Full),
            "entry" | "wejscie" => Some(ActionMode::EntryOnly),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ActionMode::Full => "full",
            ActionMode::EntryOnly => "entry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Hold,
    CloseAll,
    ClosePartial,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SlAction {
    Keep,
    /// Nowy SL w odległości `n × ATR` od BIEŻĄCEJ ceny.
    ///
    /// Parametryzacja odległością od ceny, a nie ułamkiem szczytu zysku, jest
    /// wyborem świadomym. Wersja „ułamek szczytu" nie potrafiła w ogóle ruszyć
    /// stopa pozycji STRATNEJ: szczyt wynosi wtedy zero, więc każdy docelowy SL
    /// wypadał na wejściu, czyli po złej stronie rynku, i broker odrzucał
    /// modyfikację. Model mógł zmniejszać otwarte ryzyko wyłącznie przez
    /// zamknięcie pozycji — a to przekreśla połowę sensu kary za ryzyko.
    /// Odległość od ceny działa tak samo dla pozycji w zysku (blokuje zysk) i
    /// pod wodą (obcina ryzyko), a zapadka w warstwie bezpieczeństwa i tak nie
    /// pozwoli poluzować już ustawionego stopa.
    Gap(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TpAction {
    Keep,
    /// nowy TP w odległości `n × ATR` od bieżącej ceny
    SetAtr(f64),
    /// zdejmij TP — pozycja staje się runnerem prowadzonym SL-em
    Drop,
}

#[derive(Debug, Clone, Copy)]
pub struct PosDecision {
    pub exit: Exit,
    /// część wolumenu do zamknięcia przy `ClosePartial`, w (0,1]
    pub partial: f64,
    pub sl: SlAction,
    pub tp: TpAction,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasketDecision {
    Hold,
    CancelPendings,
    /// dołóż limit: odległość w ATR, mnożnik lota, cel w ATR
    AddPending {
        dist_atr: f64,
        lot_mult: f64,
        tp_atr: f64,
    },
    CloseAll,
    /// Przestaw wejście koszyka: skasuj limity silnika i postaw własne.
    ///
    /// To jest najważniejsza akcja w całym modelu, bo jako jedyna dotyka
    /// GEOMETRII wejścia — a w tym kanale geometria jest całą grą. Strefa ma
    /// medianę 5 $; wejście przy krawędzi płytkiej daje R:R 0.5, przy głębokiej
    /// R:R 8.0. Silnik domyślnie stawia limit na krawędzi płytkiej.
    ///
    /// * `depth` — 0 = krawędź gorsza (jak silnik), 1 = krawędź lepsza,
    ///   powyżej 1 = poniżej strefy (jeszcze lepsza cena, ale rzadszy fill),
    ///   w jednostkach szerokości strefy,
    /// * `sl_mult` — odległość SL od wejścia w szerokościach strefy,
    /// * `units` — ile jednostek postawić,
    /// * `tp_frac` — który cel z drabinki sygnału wziąć (0 = najbliższy).
    Reposition {
        depth: f64,
        sl_mult: f64,
        units: u32,
        tp_frac: f64,
    },
    /// Wstrzymaj otwieranie czegokolwiek na `hours` godzin.
    ///
    /// Jawna akcja zamiast wymuszania przerwy kasowaniem limitów. Sterowanie
    /// pośrednie — „kasuj limity, aż przestaną się realizować" — jest bardzo
    /// zaszumione: model musiał trafiać w setki pojedynczych zleceń, żeby
    /// wyrazić jedną myśl „dziś nie handlujemy". Tu wyraża ją raz.
    Pause {
        hours: f64,
    },
}

#[inline]
fn sigmoid(x: f32) -> f64 {
    1.0 / (1.0 + (-(x as f64)).exp())
}

#[inline]
fn argmax3(o: &[f32], a: usize, b: usize, c: usize) -> usize {
    let mut best = a;
    if o[b] > o[best] {
        best = b;
    }
    if o[c] > o[best] {
        best = c;
    }
    best
}

/// Dekodowanie wyjść sieci pozycji na akcję.
///
/// Grupy są rozłączne i rozstrzygane przez `argmax`, nie przez próg — dzięki
/// temu decyzja jest deterministyczna i niewrażliwa na skalę wyjść.
pub fn decode_pos(o: &[f32]) -> PosDecision {
    debug_assert_eq!(o.len(), POS_OUT);
    let exit = match argmax3(o, O_HOLD, O_CLOSE, O_PARTIAL) {
        O_CLOSE => Exit::CloseAll,
        O_PARTIAL => Exit::ClosePartial,
        _ => Exit::Hold,
    };
    // 10 %..90 % wolumenu — częściowe zamknięcie nigdy nie udaje pełnego
    let partial = 0.10 + 0.80 * sigmoid(o[O_PART_FRAC]);
    let sl = if o[O_SL_SET] > o[O_SL_KEEP] {
        SlAction::Gap(0.25 + 7.75 * sigmoid(o[O_SL_GAP]))
    } else {
        SlAction::Keep
    };
    let tp = match argmax3(o, O_TP_KEEP, O_TP_SET, O_TP_DROP) {
        O_TP_SET => TpAction::SetAtr(0.5 + 7.5 * sigmoid(o[O_TP_MULT])),
        O_TP_DROP => TpAction::Drop,
        _ => TpAction::Keep,
    };
    PosDecision {
        exit,
        partial,
        sl,
        tp,
    }
}

pub fn decode_bsk(o: &[f32]) -> BasketDecision {
    decode_bsk_mode(o, ActionMode::Full)
}

/// Dekodowanie z uwzględnieniem zawężonej przestrzeni akcji.
///
/// W trybie `EntryOnly` do wyboru są wyłącznie „nic nie rób" i „przestaw
/// wejście" — pozostałe wyjścia sieci nie biorą udziału w `argmax`, więc nie
/// wnoszą szumu do poszukiwania.
pub fn decode_bsk_mode(o: &[f32], mode: ActionMode) -> BasketDecision {
    debug_assert_eq!(o.len(), BSK_OUT);
    let dozwolone: &[usize] = match mode {
        ActionMode::Full => &[B_CANCEL, B_ADD, B_CLOSE, B_REPOS, B_PAUSE],
        ActionMode::EntryOnly => &[B_REPOS, B_PAUSE],
    };
    let mut best = B_HOLD;
    for &i in dozwolone {
        if o[i] > o[best] {
            best = i;
        }
    }
    match best {
        B_CANCEL => BasketDecision::CancelPendings,
        B_CLOSE => BasketDecision::CloseAll,
        B_ADD => BasketDecision::AddPending {
            dist_atr: 0.3 + 4.0 * sigmoid(o[B_ADD_DIST]),
            lot_mult: 0.5 + 2.5 * sigmoid(o[B_ADD_LOT]),
            tp_atr: 0.5 + 5.0 * sigmoid(o[B_ADD_TP]),
        },
        B_REPOS => BasketDecision::Reposition {
            depth: 1.7 * sigmoid(o[B_DEPTH]),
            sl_mult: 0.2 + 2.8 * sigmoid(o[B_SL]),
            units: 1 + (3.0 * sigmoid(o[B_UNITS])).floor().min(2.0) as u32,
            tp_frac: sigmoid(o[B_ADD_TP]),
        },
        B_PAUSE => BasketDecision::Pause {
            hours: 0.5 + 47.5 * sigmoid(o[B_PAUSE_H]),
        },
        _ => BasketDecision::Hold,
    }
}

// ============================================================
//  SIEĆ
// ============================================================

/// Bufory robocze inferencji. Jeden na wątek, alokowane raz.
pub struct Scratch {
    a: Vec<f32>,
    b: Vec<f32>,
    pub out: Vec<f32>,
}

impl Scratch {
    pub fn for_net(m: &Mlp) -> Self {
        let h = m.dims.iter().copied().max().unwrap_or(1);
        Scratch {
            a: vec![0.0; h],
            b: vec![0.0; h],
            out: vec![0.0; *m.dims.last().unwrap_or(&1)],
        }
    }
}

/// Perceptron wielowarstwowy. Wagi wierszami: `w[l][o * in + i]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mlp {
    pub dims: Vec<usize>,
    pub w: Vec<Vec<f32>>,
    pub b: Vec<Vec<f32>>,
}

impl Mlp {
    /// Inicjalizacja: warstwy ukryte losowo (Xavier), warstwa wyjściowa
    /// **wyzerowana** z zadanymi biasami.
    ///
    /// To nie jest kosmetyka. Zerowa warstwa wyjściowa sprawia, że model na
    /// starcie realizuje dokładnie politykę „nic nie rób" (biasy ustawiają
    /// `Hold`), czyli startujemy od czytelnej linii bazowej: sygnał gra swoim
    /// SL/TP. Jednocześnie warstwy ukryte są niezerowe, więc perturbacja
    /// ostatniej warstwy daje efekt PIERWSZEGO rzędu — gdyby cała sieć była
    /// wyzerowana, ewolucja przez wiele pokoleń nie miałaby czego uczepić.
    pub fn init<R: Rng>(dims: &[usize], out_bias: &[f32], rng: &mut R) -> Self {
        assert!(dims.len() >= 2);
        let n = dims.len() - 1;
        let mut w = Vec::with_capacity(n);
        let mut b = Vec::with_capacity(n);
        for l in 0..n {
            let (i, o) = (dims[l], dims[l + 1]);
            let last = l == n - 1;
            let lim = (6.0 / (i + o) as f32).sqrt();
            let mut wl = vec![0.0f32; i * o];
            if !last {
                for x in wl.iter_mut() {
                    *x = rng.gen_range(-lim..lim);
                }
            }
            let mut bl = vec![0.0f32; o];
            if last {
                for (k, v) in out_bias.iter().enumerate().take(o) {
                    bl[k] = *v;
                }
            }
            w.push(wl);
            b.push(bl);
        }
        Mlp {
            dims: dims.to_vec(),
            w,
            b,
        }
    }

    pub fn n_params(&self) -> usize {
        self.w.iter().map(|x| x.len()).sum::<usize>()
            + self.b.iter().map(|x| x.len()).sum::<usize>()
    }

    pub fn get_params(&self, out: &mut Vec<f32>) {
        for l in 0..self.w.len() {
            out.extend_from_slice(&self.w[l]);
            out.extend_from_slice(&self.b[l]);
        }
    }

    pub fn set_params(&mut self, p: &[f32]) {
        let mut k = 0;
        for l in 0..self.w.len() {
            let n = self.w[l].len();
            self.w[l].copy_from_slice(&p[k..k + n]);
            k += n;
            let m = self.b[l].len();
            self.b[l].copy_from_slice(&p[k..k + m]);
            k += m;
        }
        debug_assert_eq!(k, p.len());
    }

    /// Przebieg w przód. Wynik ląduje w `s.out`.
    ///
    /// Aktywacja `tanh` w warstwach ukrytych jest wyborem celowym: ogranicza
    /// każdą aktywację do ±1, więc nawet skrajne wejście nie potrafi rozsadzić
    /// sieci ani wyprodukować `inf`.
    pub fn forward(&self, x: &[f32], s: &mut Scratch) {
        debug_assert_eq!(x.len(), self.dims[0]);
        let n = self.w.len();
        s.a[..x.len()].copy_from_slice(x);
        for l in 0..n {
            let (di, dof) = (self.dims[l], self.dims[l + 1]);
            if l == n - 1 {
                layer(
                    &self.w[l],
                    &self.b[l],
                    &s.a[..di],
                    &mut s.out[..dof],
                    di,
                    false,
                );
            } else {
                layer(
                    &self.w[l],
                    &self.b[l],
                    &s.a[..di],
                    &mut s.b[..dof],
                    di,
                    true,
                );
                std::mem::swap(&mut s.a, &mut s.b);
            }
        }
    }
}

#[inline]
fn layer(w: &[f32], bias: &[f32], src: &[f32], dst: &mut [f32], di: usize, act: bool) {
    for (o, d) in dst.iter_mut().enumerate() {
        let row = &w[o * di..o * di + di];
        let mut acc = bias[o];
        for i in 0..di {
            acc += row[i] * src[i];
        }
        *d = if act { acc.tanh() } else { acc };
    }
}

// ============================================================
//  POLITYKA
// ============================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Policy {
    pub pos: Mlp,
    pub bsk: Mlp,
}

/// Bias warstwy wyjściowej sieci pozycji: „trzymaj, nie ruszaj SL, nie ruszaj TP".
/// Kolejność wg stałych `O_*` — jedynki stoją na `O_HOLD`, `O_SL_KEEP`, `O_TP_KEEP`.
const POS_BIAS: [f32; POS_OUT] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// Bias warstwy wyjściowej sieci koszyka: jedynka na `B_HOLD`.
const BSK_BIAS: [f32; BSK_OUT] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

impl Policy {
    pub fn init<R: Rng>(hidden_pos: &[usize], hidden_bsk: &[usize], rng: &mut R) -> Self {
        let mut dp = vec![POS_IN];
        dp.extend_from_slice(hidden_pos);
        dp.push(POS_OUT);
        let mut db = vec![BSK_IN];
        db.extend_from_slice(hidden_bsk);
        db.push(BSK_OUT);
        Policy {
            pos: Mlp::init(&dp, &POS_BIAS, rng),
            bsk: Mlp::init(&db, &BSK_BIAS, rng),
        }
    }

    pub fn n_params(&self) -> usize {
        self.pos.n_params() + self.bsk.n_params()
    }

    pub fn get_params(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(self.n_params());
        self.pos.get_params(&mut v);
        self.bsk.get_params(&mut v);
        v
    }

    pub fn set_params(&mut self, p: &[f32]) {
        let n = self.pos.n_params();
        self.pos.set_params(&p[..n]);
        self.bsk.set_params(&p[n..]);
    }
}

// ============================================================
//  MODEL (to, co ląduje w JSON-ie)
// ============================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainMeta {
    pub algo: String,
    pub generations: usize,
    pub pop: usize,
    pub seed: u64,
    pub sigma: f32,
    pub lr: f32,
    pub windows: Vec<String>,
    pub start_balance: f64,
    pub decision_interval_s: f64,
    /// Konfiguracja SILNIKA, przy której model był trenowany.
    ///
    /// Model zarządza tym, co silnik otworzy — więc model wytrenowany przy
    /// celach „TP1" zachowuje się inaczej, gdy silnik nada pozycjom najdalszy
    /// cel. Bez zapisania tej informacji nie da się później stwierdzić, czy
    /// wdrożenie odpowiada treningowi.
    pub engine_tp_mode: String,
    pub split: String,
    pub reward: crate::reward::RewardWeights,
}

impl Default for TrainMeta {
    fn default() -> Self {
        TrainMeta {
            algo: "none".into(),
            generations: 0,
            pop: 0,
            seed: 0,
            sigma: 0.0,
            lr: 0.0,
            windows: Vec::new(),
            start_balance: 0.0,
            decision_interval_s: 2.0,
            engine_tp_mode: String::new(),
            split: String::new(),
            reward: crate::reward::RewardWeights::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrainScore {
    pub fitness: f64,
    pub pnl: f64,
    pub max_dd: f64,
    pub profit_factor: f64,
    pub trades: u32,
    pub win_rate: f64,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub format: u32,
    pub name: String,
    pub created: String,
    pub n_global: usize,
    pub n_basket: usize,
    pub n_position: usize,
    pub feature_names: Vec<String>,
    pub policy: Policy,
    pub safety: SafetyCfg,
    /// zakres akcji, jaki model dostaje do ręki
    #[serde(default)]
    pub actions: ActionMode,
    #[serde(default)]
    pub train: TrainMeta,
    #[serde(default)]
    pub score: TrainScore,
}

pub const FORMAT: u32 = 1;

impl Model {
    pub fn new(policy: Policy, safety: SafetyCfg, name: &str) -> Self {
        Model {
            format: FORMAT,
            name: name.to_string(),
            created: String::new(),
            n_global: crate::obs::G_DIM,
            n_basket: crate::obs::B_DIM,
            n_position: crate::obs::P_DIM,
            feature_names: crate::obs::feature_names(),
            policy,
            safety,
            actions: ActionMode::default(),
            train: TrainMeta::default(),
            score: TrainScore::default(),
        }
    }

    /// Model startowy: polityka „nic nie rób" plus losowe warstwy ukryte.
    pub fn fresh(hidden_pos: &[usize], hidden_bsk: &[usize], seed: u64, safety: SafetyCfg) -> Self {
        use rand::SeedableRng;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        Model::new(
            Policy::init(hidden_pos, hidden_bsk, &mut rng),
            safety,
            "fresh",
        )
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let txt = std::fs::read_to_string(path.as_ref())?;
        let m: Model = serde_json::from_str(&txt)?;
        m.validate()?;
        Ok(m)
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let mut m = self.clone();
        m.sanitize()?;
        if let Some(d) = path.as_ref().parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path.as_ref(), serde_json::to_string_pretty(&m)?)?;
        Ok(())
    }

    /// Usuwa wartości, których JSON nie potrafi wyrazić.
    ///
    /// `serde_json` odmawia zapisu `NaN` i `±inf` — i słusznie, bo w JSON-ie
    /// takich literałów nie ma. Bez tego kroku cały wielogodzinny trening kończy
    /// się błędem przy zapisie, a model przepada. Dwa realne źródła:
    ///  * `profit_factor` = ∞, gdy okno nie miało ANI JEDNEJ straty,
    ///  * rozbiegane wagi, jeśli krok ewolucji wyprodukował `NaN`.
    ///
    /// Pierwsze naprawiamy (∞ → −1 jako umowne „brak strat"), drugie jest błędem
    /// treningu i musi być głośne, a nie zapisane po cichu.
    pub fn sanitize(&mut self) -> anyhow::Result<()> {
        use anyhow::bail;
        let bad = self
            .policy
            .get_params()
            .iter()
            .filter(|x| !x.is_finite())
            .count();
        if bad > 0 {
            bail!("model zawiera {bad} niepoprawnych wag (NaN/inf) — trening się rozbiegł");
        }
        for v in [
            &mut self.score.fitness,
            &mut self.score.pnl,
            &mut self.score.max_dd,
            &mut self.score.win_rate,
        ] {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
        if !self.score.profit_factor.is_finite() {
            // −1 czyta się jednoznacznie: „nie było strat, iloraz nieokreślony"
            self.score.profit_factor = -1.0;
        }
        Ok(())
    }

    /// Model z innym zestawem cech to model, który po cichu liczyłby bzdury.
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        if self.format != FORMAT {
            bail!("nieznany format modelu: {}", self.format);
        }
        if self.policy.pos.dims.first() != Some(&POS_IN)
            || self.policy.pos.dims.last() != Some(&POS_OUT)
        {
            bail!(
                "sieć pozycji ma wymiary {:?}, oczekiwano {POS_IN}→…→{POS_OUT}",
                self.policy.pos.dims
            );
        }
        if self.policy.bsk.dims.first() != Some(&BSK_IN)
            || self.policy.bsk.dims.last() != Some(&BSK_OUT)
        {
            bail!(
                "sieć koszyka ma wymiary {:?}, oczekiwano {BSK_IN}→…→{BSK_OUT}",
                self.policy.bsk.dims
            );
        }
        if self.n_global != crate::obs::G_DIM
            || self.n_basket != crate::obs::B_DIM
            || self.n_position != crate::obs::P_DIM
        {
            bail!("model zbudowany na innym zestawie cech niż bieżący kod");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn policy() -> Policy {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        Policy::init(&[24, 16], &[16, 12], &mut rng)
    }

    #[test]
    fn biasy_stoja_na_wlasciwych_indeksach() {
        // tablice są pisane literalnie — ten test pilnuje, żeby zmiana kolejności
        // wyjść nie przestawiła po cichu domyślnego zachowania modelu
        assert_eq!(POS_BIAS.iter().filter(|x| **x != 0.0).count(), 3);
        assert_eq!(POS_BIAS[O_HOLD], 1.0);
        assert_eq!(POS_BIAS[O_SL_KEEP], 1.0);
        assert_eq!(POS_BIAS[O_TP_KEEP], 1.0);
        assert_eq!(BSK_BIAS.iter().filter(|x| **x != 0.0).count(), 1);
        assert_eq!(BSK_BIAS[B_HOLD], 1.0);
    }

    #[test]
    fn forward_jest_deterministyczny() {
        let p = policy();
        let x: Vec<f32> = (0..POS_IN).map(|i| ((i % 7) as f32 - 3.0) * 0.31).collect();
        let mut s1 = Scratch::for_net(&p.pos);
        let mut s2 = Scratch::for_net(&p.pos);
        p.pos.forward(&x, &mut s1);
        for _ in 0..50 {
            p.pos.forward(&x, &mut s2);
        }
        assert_eq!(s1.out, s2.out);
        assert!(s1.out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn swieza_polityka_nic_nie_robi() {
        // model startowy musi być czytelną linią bazową: żadnych akcji
        let p = policy();
        let mut sp = Scratch::for_net(&p.pos);
        let mut sb = Scratch::for_net(&p.bsk);
        for k in 0..64 {
            let x: Vec<f32> = (0..POS_IN)
                .map(|i| (((i * 13 + k * 7) % 17) as f32 - 8.0) * 0.5)
                .collect();
            p.pos.forward(&x, &mut sp);
            let d = decode_pos(&sp.out);
            assert_eq!(d.exit, Exit::Hold);
            assert_eq!(d.sl, SlAction::Keep);
            assert_eq!(d.tp, TpAction::Keep);
            p.bsk.forward(&x[..BSK_IN], &mut sb);
            assert_eq!(decode_bsk(&sb.out), BasketDecision::Hold);
        }
    }

    #[test]
    fn parametry_round_trip() {
        let mut p = policy();
        let v = p.get_params();
        assert_eq!(v.len(), p.n_params());
        let shifted: Vec<f32> = v.iter().map(|x| x + 0.125).collect();
        p.set_params(&shifted);
        assert_eq!(p.get_params(), shifted);
    }

    #[test]
    fn zapis_radzi_sobie_z_nieskonczonym_profit_factor() {
        // REGRESJA: `serde_json` odmawia zapisu ∞, więc bez sanityzacji trening
        // bez ani jednej straty kończył się błędem przy zapisie modelu.
        let mut m = Model::new(policy(), SafetyCfg::default(), "test");
        m.score.profit_factor = f64::INFINITY;
        m.score.fitness = f64::NAN;
        let f = std::env::temp_dir().join("conduit_ai_test_model.json");
        m.save(&f).expect("zapis modelu z ∞ musi się udać");
        let back = Model::load(&f).unwrap();
        assert_eq!(back.score.profit_factor, -1.0);
        assert_eq!(back.score.fitness, 0.0);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn zapis_odmawia_gdy_wagi_sa_nan() {
        let mut m = Model::new(policy(), SafetyCfg::default(), "test");
        let mut p = m.policy.get_params();
        p[17] = f32::NAN;
        m.policy.set_params(&p);
        let f = std::env::temp_dir().join("conduit_ai_test_nan.json");
        assert!(
            m.save(&f).is_err(),
            "model z NaN nie może zostać zapisany po cichu"
        );
    }

    #[test]
    fn model_serializuje_sie_bezstratnie() {
        let m = Model::new(policy(), SafetyCfg::default(), "test");
        let js = serde_json::to_string(&m).unwrap();
        let back: Model = serde_json::from_str(&js).unwrap();
        back.validate().unwrap();
        assert_eq!(back.policy.get_params(), m.policy.get_params());
        assert_eq!(back.feature_names, m.feature_names);
        assert_eq!(back.policy.pos.dims, m.policy.pos.dims);

        // i identycznie liczy
        let x: Vec<f32> = (0..POS_IN).map(|i| (i as f32 * 0.017).sin()).collect();
        let mut s1 = Scratch::for_net(&m.policy.pos);
        let mut s2 = Scratch::for_net(&back.policy.pos);
        m.policy.pos.forward(&x, &mut s1);
        back.policy.pos.forward(&x, &mut s2);
        assert_eq!(s1.out, s2.out);
    }

    #[test]
    fn skrajne_wejscia_nie_daja_nan() {
        let mut p = policy();
        // wagi rozdmuchane 1000×
        let big: Vec<f32> = p.get_params().iter().map(|x| x * 1000.0 + 500.0).collect();
        p.set_params(&big);
        let mut s = Scratch::for_net(&p.pos);
        for v in [-8.0f32, 8.0, 0.0] {
            let x = vec![v; POS_IN];
            p.pos.forward(&x, &mut s);
            assert!(s.out.iter().all(|o| o.is_finite()), "wyjście {:?}", s.out);
            let d = decode_pos(&s.out);
            assert!(d.partial > 0.0 && d.partial <= 1.0);
        }
    }
}

// ============================================================
//  PROPAGACJA WSTECZNA
// ============================================================
//
// Sieć była pisana wyłącznie „w przód", bo strategia ewolucyjna nie potrzebuje
// gradientów. Uczenie z naśladowania odwraca to założenie: entropia krzyżowa i
// błąd średniokwadratowy wymagają pochodnych. Poniżej minimalny, ale kompletny
// backprop dla tej samej struktury — warstwy ukryte `tanh`, wyjście liniowe.
//
// Poprawność jest pilnowana testem gradientu numerycznego, a nie wiarą w to, że
// wzory zostały przepisane bez literówki.

/// Gradienty o kształcie wag.
#[derive(Clone, Debug)]
pub struct Grads {
    pub w: Vec<Vec<f32>>,
    pub b: Vec<Vec<f32>>,
}

impl Grads {
    pub fn zeros_like(m: &Mlp) -> Self {
        Grads {
            w: m.w.iter().map(|x| vec![0.0; x.len()]).collect(),
            b: m.b.iter().map(|x| vec![0.0; x.len()]).collect(),
        }
    }
    pub fn clear(&mut self) {
        for v in self.w.iter_mut().chain(self.b.iter_mut()) {
            v.iter_mut().for_each(|x| *x = 0.0);
        }
    }
    /// Skalowanie — po zsumowaniu gradientów z porcji dzielimy przez jej rozmiar.
    pub fn scale(&mut self, k: f32) {
        for v in self.w.iter_mut().chain(self.b.iter_mut()) {
            v.iter_mut().for_each(|x| *x *= k);
        }
    }
    pub fn flat(&self) -> Vec<f32> {
        let mut out = Vec::new();
        for l in 0..self.w.len() {
            out.extend_from_slice(&self.w[l]);
            out.extend_from_slice(&self.b[l]);
        }
        out
    }
}

/// Bufory uczenia: aktywacje każdej warstwy (potrzebne wstecz) + delty.
pub struct BackScratch {
    /// `acts[0]` to wejście, `acts[l+1]` to wyjście warstwy `l`
    pub acts: Vec<Vec<f32>>,
    d_cur: Vec<f32>,
    d_prev: Vec<f32>,
}

impl BackScratch {
    pub fn for_net(m: &Mlp) -> Self {
        let h = m.dims.iter().copied().max().unwrap_or(1);
        BackScratch {
            acts: m.dims.iter().map(|d| vec![0.0; *d]).collect(),
            d_cur: vec![0.0; h],
            d_prev: vec![0.0; h],
        }
    }
    /// Wyjście sieci po ostatnim `forward_cached`.
    pub fn out(&self) -> &[f32] {
        self.acts.last().unwrap()
    }
}

impl Mlp {
    /// Przebieg w przód zapamiętujący aktywacje — wymagany przed `backward`.
    pub fn forward_cached(&self, x: &[f32], s: &mut BackScratch) {
        debug_assert_eq!(x.len(), self.dims[0]);
        s.acts[0].copy_from_slice(x);
        let n = self.w.len();
        for l in 0..n {
            let (di, dof) = (self.dims[l], self.dims[l + 1]);
            let last = l == n - 1;
            let (src, dst) = s.acts.split_at_mut(l + 1);
            layer(
                &self.w[l],
                &self.b[l],
                &src[l][..di],
                &mut dst[0][..dof],
                di,
                !last,
            );
        }
    }

    /// Propagacja wsteczna. `dout` to pochodna straty po WYJŚCIU sieci.
    /// Gradienty są DODAWANE do `g`, więc porcję sumuje się przez wielokrotne
    /// wywołanie, a potem dzieli przez jej rozmiar.
    pub fn backward(&self, s: &mut BackScratch, dout: &[f32], g: &mut Grads) {
        let n = self.w.len();
        debug_assert_eq!(dout.len(), self.dims[n]);
        s.d_cur[..dout.len()].copy_from_slice(dout);

        for l in (0..n).rev() {
            let (di, dof) = (self.dims[l], self.dims[l + 1]);
            // gradienty wag i biasów tej warstwy
            for o in 0..dof {
                let d = s.d_cur[o];
                if d != 0.0 {
                    let row = o * di;
                    for i in 0..di {
                        g.w[l][row + i] += d * s.acts[l][i];
                    }
                }
                g.b[l][o] += d;
            }
            if l == 0 {
                break;
            }
            // delta w dół: przez wagi, potem przez pochodną tanh (1 − a²)
            for i in 0..di {
                let mut acc = 0.0f32;
                for o in 0..dof {
                    acc += s.d_cur[o] * self.w[l][o * di + i];
                }
                let a = s.acts[l][i];
                s.d_prev[i] = acc * (1.0 - a * a);
            }
            s.d_cur[..di].copy_from_slice(&s.d_prev[..di]);
        }
    }
}

/// Strata kwadratowa. Zwraca wartość i wpisuje pochodną do `dout`.
pub fn mse(pred: &[f32], target: &[f32], dout: &mut [f32]) -> f32 {
    let n = pred.len() as f32;
    let mut s = 0.0;
    for i in 0..pred.len() {
        let e = pred[i] - target[i];
        s += e * e;
        dout[i] = 2.0 * e / n;
    }
    s / n
}

/// Entropia krzyżowa na logitach (numerycznie stabilna).
/// `target` w {0,1}. Pochodna po logicie to po prostu `sigmoid(z) − y`.
pub fn bce_logits(pred: &[f32], target: &[f32], dout: &mut [f32]) -> f32 {
    let n = pred.len() as f32;
    let mut s = 0.0;
    for i in 0..pred.len() {
        let z = pred[i];
        let y = target[i];
        // log(1+e^z) liczone stabilnie dla obu znaków z
        let softplus = if z > 0.0 {
            z + (1.0 + (-z).exp()).ln()
        } else {
            (1.0 + z.exp()).ln()
        };
        s += softplus - y * z;
        dout[i] = (1.0 / (1.0 + (-z).exp()) - y) / n;
    }
    s / n
}

#[cfg(test)]
mod grad_tests {
    use super::*;
    use rand::SeedableRng;

    fn siec(dims: &[usize]) -> Mlp {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(11);
        // biasy wyjścia losowe, żeby test nie trafił w przypadkiem wygodny punkt
        let ob: Vec<f32> = (0..*dims.last().unwrap())
            .map(|i| (i as f32 * 0.37).sin())
            .collect();
        let mut m = Mlp::init(dims, &ob, &mut rng);
        // `init` zeruje warstwę wyjściową — do testu gradientu potrzebujemy jej niezerowej
        let n = m.w.len() - 1;
        for (k, v) in m.w[n].iter_mut().enumerate() {
            *v = ((k as f32 * 0.19).cos()) * 0.5;
        }
        m
    }

    /// Strata na potrzeby testu: MSE względem ustalonego celu.
    fn strata(m: &Mlp, x: &[f32], t: &[f32]) -> f32 {
        let mut s = Scratch::for_net(m);
        m.forward(x, &mut s);
        let n = t.len() as f32;
        s.out
            .iter()
            .zip(t)
            .map(|(p, y)| (p - y) * (p - y))
            .sum::<f32>()
            / n
    }

    #[test]
    fn gradient_zgadza_sie_z_numerycznym() {
        // Jedyny test, który naprawdę weryfikuje backprop. Bez niego literówka w
        // indeksie wag daje sieć, która uczy się „prawie" i nikt tego nie zauważa.
        let dims = [6usize, 5, 4, 3];
        let mut m = siec(&dims);
        let x: Vec<f32> = (0..dims[0])
            .map(|i| ((i as f32 + 1.0) * 0.41).sin())
            .collect();
        let t: Vec<f32> = (0..3).map(|i| (i as f32 * 0.7).cos()).collect();

        let mut bs = BackScratch::for_net(&m);
        let mut g = Grads::zeros_like(&m);
        m.forward_cached(&x, &mut bs);
        let mut dout = vec![0.0f32; 3];
        mse(bs.out(), &t, &mut dout);
        m.backward(&mut bs, &dout, &mut g);

        // różnica centralna po każdej wadze i każdym biasie
        const H: f32 = 1e-3;
        let mut sprawdzone = 0;
        for l in 0..m.w.len() {
            for k in 0..m.w[l].len() {
                let orig = m.w[l][k];
                m.w[l][k] = orig + H;
                let a = strata(&m, &x, &t);
                m.w[l][k] = orig - H;
                let b = strata(&m, &x, &t);
                m.w[l][k] = orig;
                let num = (a - b) / (2.0 * H);
                let ana = g.w[l][k];
                let skala = num.abs().max(ana.abs()).max(1e-3);
                assert!(
                    (num - ana).abs() / skala < 2e-2,
                    "warstwa {l} waga {k}: numerycznie {num:.6}, analitycznie {ana:.6}"
                );
                sprawdzone += 1;
            }
            for k in 0..m.b[l].len() {
                let orig = m.b[l][k];
                m.b[l][k] = orig + H;
                let a = strata(&m, &x, &t);
                m.b[l][k] = orig - H;
                let b = strata(&m, &x, &t);
                m.b[l][k] = orig;
                let num = (a - b) / (2.0 * H);
                let ana = g.b[l][k];
                let skala = num.abs().max(ana.abs()).max(1e-3);
                assert!(
                    (num - ana).abs() / skala < 2e-2,
                    "warstwa {l} bias {k}: numerycznie {num:.6}, analitycznie {ana:.6}"
                );
                sprawdzone += 1;
            }
        }
        assert!(sprawdzone > 70, "sprawdzono tylko {sprawdzone} parametrów");
    }

    #[test]
    fn forward_cached_daje_to_samo_co_forward() {
        // dwie ścieżki w przód muszą się zgadzać co do bitu, inaczej gradient
        // dotyczy innej sieci niż ta, która handluje
        let m = siec(&[7, 6, 5, 4]);
        let x: Vec<f32> = (0..7).map(|i| (i as f32 * 0.23).sin() * 3.0).collect();
        let mut s = Scratch::for_net(&m);
        let mut bs = BackScratch::for_net(&m);
        m.forward(&x, &mut s);
        m.forward_cached(&x, &mut bs);
        assert_eq!(&s.out[..], bs.out());
    }

    #[test]
    fn entropia_krzyzowa_ma_poprawna_pochodna() {
        // pochodna po logicie to sigmoid(z) − y; sprawdzamy numerycznie
        let pred = [0.8f32, -1.4, 0.0];
        let targ = [1.0f32, 0.0, 1.0];
        let mut d = [0.0f32; 3];
        bce_logits(&pred, &targ, &mut d);
        const H: f32 = 1e-3;
        for i in 0..3 {
            let mut p = pred;
            let mut tmp = [0.0f32; 3];
            p[i] += H;
            let a = bce_logits(&p, &targ, &mut tmp);
            p[i] -= 2.0 * H;
            let b = bce_logits(&p, &targ, &mut tmp);
            let num = (a - b) / (2.0 * H);
            assert!((num - d[i]).abs() < 1e-3, "i={i}: {num} vs {}", d[i]);
        }
    }

    #[test]
    fn siec_uczy_sie_prostej_zaleznosci() {
        // Test końcowy: czy tym backpropem da się w ogóle czegoś nauczyć.
        // Zadanie nieliniowe (XOR), więc sukces wyklucza przypadek.
        let mut m = siec(&[2, 8, 8, 1]);
        let dane = [
            ([0.0f32, 0.0], 0.0f32),
            ([1.0, 0.0], 1.0),
            ([0.0, 1.0], 1.0),
            ([1.0, 1.0], 0.0),
        ];
        let mut bs = BackScratch::for_net(&m);
        let mut g = Grads::zeros_like(&m);
        let mut start = 0.0;
        let mut koniec = 0.0;
        // krok dobrany ostrożnie: przy 0.5 sieć rozbiegała się do NaN, bo warstwa
        // wyjściowa jest liniowa i nie ma czego nasycić
        let lr = 0.05f32;
        for epoka in 0..8000 {
            g.clear();
            let mut l = 0.0;
            for (x, y) in &dane {
                m.forward_cached(x, &mut bs);
                let mut dout = [0.0f32; 1];
                l += mse(bs.out(), &[*y], &mut dout);
                m.backward(&mut bs, &dout, &mut g);
            }
            g.scale(1.0 / dane.len() as f32);
            for li in 0..m.w.len() {
                for k in 0..m.w[li].len() {
                    m.w[li][k] -= lr * g.w[li][k];
                }
                for k in 0..m.b[li].len() {
                    m.b[li][k] -= lr * g.b[li][k];
                }
            }
            if epoka == 0 {
                start = l / 4.0;
            }
            koniec = l / 4.0;
        }
        assert!(koniec < start * 0.05, "strata {start:.4} → {koniec:.4}");
        assert!(koniec < 0.01, "sieć nie nauczyła się XOR: {koniec:.4}");
    }
}
