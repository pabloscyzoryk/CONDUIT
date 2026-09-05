
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    pub nazwa: String,
    pub parser: String,
    #[serde(default)]
    pub opis: String,
}

impl Format {
    pub fn nowy(nazwa: &str, parser: &str, opis: &str) -> Self {
        Format {
            nazwa: nazwa.into(),
            parser: parser.into(),
            opis: opis.into(),
        }
    }
}

pub fn formaty_wbudowane() -> Vec<Format> {
    ["ATFX", "Synergy", "ZEN", "PULSEX", "NOVA", "TWP", "DANGER", "CLUB1", "STORM"]
        .into_iter()
        .map(|name| {
            Format::nowy(
                name,
                "atfx",
                "Built-in signal syntax. Configure the channel or topic identifier yourself.",
            )
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PulapyGlobalne {
    pub max_pozycji: u32,
    pub max_koszykow: u32,
    pub max_lotow: f64,
    pub max_lotow_kierunkowo: f64,
    pub max_ryzyko_pct: f64,

    pub max_dd_pct: f64,
    pub max_dd_usd: f64,
    pub podloga_equity_usd: f64,

    pub cel_dnia_usd: f64,
    pub cel_dnia_pct: f64,
    pub cel_dnia_zamyka: bool,
    pub limit_straty_dnia_usd: f64,
    pub limit_straty_dnia_pct: f64,

    pub blokuj_przeciwne_kierunki: bool,
    pub pauza_po_stratach_n: u32,
    pub pauza_po_stratach_min: f64,
}

impl PulapyGlobalne {
    pub fn sufit_u32(globalny: u32, presetu: u32) -> u32 {
        match (globalny, presetu) {
            (0, p) => p,
            (g, 0) => g,
            (g, p) => g.min(p),
        }
    }

    pub fn sufit_f64(globalny: f64, presetu: f64) -> f64 {
        match (globalny > 0.0, presetu > 0.0) {
            (false, _) => presetu,
            (true, false) => globalny,
            (true, true) => globalny.min(presetu),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lancuch {
    pub nazwa: String,
    #[serde(default)]
    pub opis: String,
    #[serde(default)]
    pub presety: BTreeMap<String, String>,
    #[serde(default)]
    pub pulapy: PulapyGlobalne,
}

impl Lancuch {
    pub fn preset_dla(&self, format: &str) -> Option<&str> {
        self.presety
            .get(format)
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lancuchy {
    pub aktywny: String,
    pub lista: Vec<Lancuch>,
}

impl Default for Lancuchy {
    fn default() -> Self {
        Lancuchy {
            aktywny: "SENTINEL-0".into(),
            lista: lancuchy_wbudowane(),
        }
    }
}

impl Lancuchy {
    pub fn aktywny(&self) -> Option<&Lancuch> {
        self.lista.iter().find(|l| l.nazwa == self.aktywny)
    }

    pub fn preset_dla(&self, format: &str) -> Option<&str> {
        self.aktywny().and_then(|l| l.preset_dla(format))
    }

    pub fn formaty_handlujace(&self) -> Vec<String> {
        match self.aktywny() {
            Some(l) => l
                .presety
                .iter()
                .filter(|(_, p)| !p.is_empty())
                .map(|(f, _)| f.clone())
                .collect(),
            None => Vec::new(),
        }
    }
}

fn lancuch(nazwa: &str, opis: &str, pary: &[(&str, &str)], pulapy: PulapyGlobalne) -> Lancuch {
    Lancuch {
        nazwa: nazwa.into(),
        opis: opis.into(),
        presety: pary
            .iter()
            .map(|(f, p)| (f.to_string(), p.to_string()))
            .collect(),
        pulapy,
    }
}

pub fn lancuchy_wbudowane() -> Vec<Lancuch> {
    vec![
        lancuch(
            "SENTINEL-0",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("Synergy", "HYPER-2"),
                ("ZEN", "FRESHQUEEN-3"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "SENTINEL-1",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("Synergy", "HYPER-2"),
                ("ATFX", ""),
                ("ZEN", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 9,
                max_koszykow: 3,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "SENTINEL-2",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("Synergy", "HYPER-2"),
                ("ZEN", "FRESHQUEEN-3"),
                ("NOVA", "QUASAR-1"),
                ("ATFX", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 18,
                max_koszykow: 5,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "SENTINEL-0C",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-3"),
                ("Synergy", "HYPER-2C"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "ZENONLY5",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-5"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENONLY7",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-7"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENONLY4",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-4"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENS03",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S03"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENS04",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S04"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENS06",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S06"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENS08",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S08"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "ZENS12",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S12"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "SENTINEL-X0",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S06"),
                ("Synergy", "HYPER-2C"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "SENTINEL-X1",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S08"),
                ("Synergy", "HYPER-2C"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "SENTINEL-X2",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FQ-S12"),
                ("Synergy", "HYPER-2C"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "ZENSKALA",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-S"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "FS-X0",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-X1-S"),
                ("Synergy", "HYPER-X2-S"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-X0C",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-X1C-S"),
                ("Synergy", "HYPER-X2C-S"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-M1-START",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M1-A"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "FS-M1-ROZWOJ",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M1-A"),
                ("Synergy", "FS-M1-B"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-M1-PELNA",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M1-A"),
                ("Synergy", "FS-M1-C"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-M2-SOLO",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", ""),
                ("Synergy", "FS-M2-SYN"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "FS-M2-ZEN-SOLO",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M2-ZEN"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "FS-M2-PARA",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M2-ZEN"),
                ("Synergy", "FS-M2-SYN"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-M2-PARA-P",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M2-ZENP"),
                ("Synergy", "FS-M2-SYN"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-M3-SOLO",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", ""),
                ("Synergy", "FS-M3-SYN"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "MONOLIT-4",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "NEWKOAN-3"),
                ("Synergy", "OMEGA-X2"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
                ("TWP", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 20,
                max_koszykow: 6,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "MONOLIT-3",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "NEWKOAN-3"),
                ("Synergy", "OMEGA-X1"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
                ("TWP", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 20,
                max_koszykow: 6,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "MONOLIT-2",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "NEWKOAN-3"),
                ("Synergy", "MONOLIT-SYN"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
                ("TWP", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 20,
                max_koszykow: 6,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "MONOLIT-1",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "NEWKOAN-3"),
                ("Synergy", "OMEGA-2"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
                ("TWP", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 20,
                max_koszykow: 6,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "OMEGA-3",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "NEWKOAN-3"),
                ("Synergy", "OMEGA-1"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
                ("TWP", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 20,
                max_koszykow: 6,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "SENTINEL-3",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "NEWKOAN-2"),
                ("Synergy", "NEWALPHA-2"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 20,
                max_koszykow: 6,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "FS-M3-PARA",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FS-M3-ZEN"),
                ("Synergy", "FS-M3-SYN"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "ZENONLY3",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("ZEN", "FRESHQUEEN-3"),
                ("Synergy", ""),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "SENTINEL-0A",
            "Built-in chain configuration. Review every risk setting before use.",
            &[
                ("Synergy", "HYPER-2"),
                ("ZEN", "FRESHQUEEN-4"),
                ("ATFX", ""),
                ("NOVA", ""),
                ("PULSEX", ""),
            ],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "PODSTAWOWY",
            "Built-in chain configuration. Review every risk setting before use.",
            &[("ATFX", "HYPER-2"), ("Synergy", "HYPER-2")],
            PulapyGlobalne {
                max_pozycji: 14,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
        lancuch(
            "TYLKO ATFX",
            "Built-in chain configuration. Review every risk setting before use.",
            &[("ATFX", "HYPER-2")],
            PulapyGlobalne::default(),
        ),
        lancuch(
            "OSTROŻNY",
            "Built-in chain configuration. Review every risk setting before use.",
            &[("ATFX", "ULTRA-X3"), ("Synergy", "ULTRA-X3")],
            PulapyGlobalne {
                max_pozycji: 12,
                max_koszykow: 4,
                blokuj_przeciwne_kierunki: true,
                ..Default::default()
            },
        ),
    ]
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kierunek {
    NizszeOstrzej,
    WyzszeOstrzej,
    PrawdaOstrzej,
}

pub const BEZPIECZNIKI_RACHUNKU: &[(&str, Kierunek)] = &[
    ("expo_cap_pct", Kierunek::NizszeOstrzej),
    ("expo_cap_ml_pct", Kierunek::WyzszeOstrzej),
    ("margin_call_level_pct", Kierunek::WyzszeOstrzej),
    ("expo_cap_close", Kierunek::PrawdaOstrzej),
    ("odlicz_kredyt", Kierunek::PrawdaOstrzej),
];

pub const POLA_PARYTETU_WYKONANIA: &[&str] = &[
    "close_receipt_reconcile",
    "closed_profit_net_costs",
    "restore_strategy_continuation",
    "order_volume_contract_v2",
    "expo_cap_pct",
    "expo_cap_ml_pct",
    "expo_cap_close",
    "expo_cap_s",
    "lot_base",
    "odlicz_kredyt",
    "credit_balance_separate",
    "kredyt_reczny",
];

#[derive(Debug, Clone, PartialEq)]
pub struct RozjazdRachunku {
    pub format: String,
    pub pole: String,
    pub presetu: String,
    pub rachunku: String,
    pub oslabia: bool,
}

impl std::fmt::Display for RozjazdRachunku {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "  {:<10} {}: {} → {}{}",
            self.format,
            self.pole,
            self.presetu,
            self.rachunku,
            if self.oslabia {
                "   ⛔ BEZPIECZNIK OSŁABIONY"
            } else {
                ""
            }
        )
    }
}

fn oslabia_bezpiecznik(
    pole: &str,
    presetu: &serde_json::Value,
    rachunku: &serde_json::Value,
) -> bool {
    let Some((_, kier)) = BEZPIECZNIKI_RACHUNKU.iter().find(|(k, _)| *k == pole) else {
        return false;
    };
    match kier {
        Kierunek::NizszeOstrzej => match (presetu.as_f64(), rachunku.as_f64()) {
            (Some(p), Some(a)) => p > 0.0 && (a <= 0.0 || a > p),
            _ => false,
        },
        Kierunek::WyzszeOstrzej => match (presetu.as_f64(), rachunku.as_f64()) {
            (Some(p), Some(a)) => p > 0.0 && (a <= 0.0 || a < p),
            _ => false,
        },
        Kierunek::PrawdaOstrzej => {
            presetu.as_bool() == Some(true) && rachunku.as_bool() == Some(false)
        }
    }
}

pub fn rozjazd_rachunku(
    nogi: &[(&str, &crate::Settings)],
    rachunek: &crate::Settings,
) -> Vec<RozjazdRachunku> {
    let mut v = Vec::new();
    let Ok(r) = serde_json::to_value(rachunek) else {
        return v;
    };
    let Some(ro) = r.as_object() else { return v };
    for (format, cfg) in nogi {
        let Ok(p) = serde_json::to_value(cfg) else {
            continue;
        };
        let Some(po) = p.as_object() else { continue };
        for k in crate::wielosilnik::POLA_RACHUNKU {
            match (po.get(*k), ro.get(*k)) {
                (Some(a), Some(b)) if a != b => v.push(RozjazdRachunku {
                    format: (*format).to_string(),
                    pole: (*k).to_string(),
                    presetu: a.to_string(),
                    rachunku: b.to_string(),
                    oslabia: oslabia_bezpiecznik(k, a, b),
                }),
                _ => {}
            }
        }
    }
    v
}

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn lancuch_bez_wpisu_nie_handluje() {
        let w = lancuchy_wbudowane();
        let l = w
            .iter()
            .find(|x| x.nazwa == "TYLKO ATFX")
            .expect("lancuch TYLKO ATFX");
        assert_eq!(l.preset_dla("ATFX"), Some("HYPER-2"));
        assert_eq!(l.preset_dla("Synergy"), None, "brak wpisu = brak handlu");
    }

    #[test]
    fn pusty_wpis_znaczy_to_samo_co_brak() {
        let mut l = Lancuch::default();
        l.presety.insert("Synergy".into(), String::new());
        assert_eq!(l.preset_dla("Synergy"), None);
    }

    #[test]
    fn kazdy_wbudowany_lancuch_wskazuje_istniejace_formaty() {
        let znane: Vec<String> = formaty_wbudowane().into_iter().map(|f| f.nazwa).collect();
        for l in lancuchy_wbudowane() {
            for f in l.presety.keys() {
                assert!(
                    znane.contains(f),
                    "łańcuch {} wskazuje nieznany format {f}",
                    l.nazwa
                );
            }
        }
    }

    #[test]
    fn sufit_zero_znaczy_bez_pulapu() {
        assert_eq!(
            PulapyGlobalne::sufit_u32(0, 9),
            9,
            "0 globalnie = rządzi preset"
        );
        assert_eq!(
            PulapyGlobalne::sufit_u32(14, 0),
            14,
            "0 w presecie = rządzi pułap"
        );
        assert_eq!(PulapyGlobalne::sufit_u32(14, 9), 9, "wygrywa NIŻSZY");
        assert_eq!(PulapyGlobalne::sufit_u32(4, 9), 4);
        assert_eq!(PulapyGlobalne::sufit_u32(0, 0), 0, "oba zera = bez limitu");
        assert_eq!(PulapyGlobalne::sufit_f64(0.0, 2.5), 2.5);
        assert_eq!(PulapyGlobalne::sufit_f64(1.5, 0.0), 1.5);
        assert_eq!(PulapyGlobalne::sufit_f64(1.5, 2.5), 1.5);
    }

    #[test]
    fn pulap_globalny_jest_nizszy_niz_suma_presetow() {
        let w = lancuchy_wbudowane();
        let l = w
            .iter()
            .find(|x| x.nazwa == "SENTINEL-0")
            .expect("lancuch SENTINEL-0");
        assert_eq!(
            l.presety.values().filter(|p| !p.is_empty()).count(),
            2,
            "SENTINEL-0 ma DWA formaty handlujace"
        );
        assert!(
            l.pulapy.max_pozycji > 0 && l.pulapy.max_pozycji < 9 * 2,
            "pułap {} nie chroni niczego przy dwóch presetach po 9 pozycji",
            l.pulapy.max_pozycji
        );
        assert!(l.pulapy.max_koszykow > 0 && l.pulapy.max_koszykow < 3 * 2);
    }


    #[test]
    fn zgodne_nogi_nie_daja_zadnego_rozjazdu() {
        let r = crate::Settings::default();
        let a = r.clone();
        let b = r.clone();
        let nogi: Vec<(&str, &crate::Settings)> = vec![("Synergy", &a), ("ZEN", &b)];
        assert!(rozjazd_rachunku(&nogi, &r).is_empty());
    }

    #[test]
    fn zdjecie_expo_cap_nodze_jest_oslabieniem() {
        let rachunek = crate::Settings::default(); // expo_cap_pct = 0.0
        let mut storm = crate::Settings::default();
        storm.expo_cap_pct = 80.0;
        let nogi: Vec<(&str, &crate::Settings)> = vec![("STORM", &storm)];
        let v = rozjazd_rachunku(&nogi, &rachunek);
        let x = v
            .iter()
            .find(|x| x.pole == "expo_cap_pct")
            .expect("rozjazd expo_cap_pct");
        assert_eq!(x.format, "STORM");
        assert_eq!(x.presetu, "80.0");
        assert_eq!(x.rachunku, "0.0");
        assert!(
            x.oslabia,
            "zdjęcie straży ekspozycji to OSŁABIENIE, nie kosmetyka"
        );
        assert!(x.to_string().contains("BEZPIECZNIK OSŁABIONY"));
    }

    #[test]
    fn ostrzejszy_rachunek_nie_jest_oslabieniem() {
        let mut rachunek = crate::Settings::default();
        rachunek.expo_cap_pct = 50.0;
        rachunek.expo_cap_ml_pct = 300.0;
        rachunek.expo_cap_close = true;
        let mut noga = crate::Settings::default();
        noga.expo_cap_pct = 80.0; // luźniej niż rachunek
        noga.expo_cap_ml_pct = 150.0; // niższy próg = później hamuje
        noga.expo_cap_close = false;
        let nogi: Vec<(&str, &crate::Settings)> = vec![("STORM", &noga)];
        let v = rozjazd_rachunku(&nogi, &rachunek);
        assert_eq!(v.len(), 3, "trzy pola się różnią i mają być WYPISANE");
        assert!(
            v.iter().all(|x| !x.oslabia),
            "rachunek ostrzejszy od presetu nie zdejmuje nodze niczego"
        );
    }

    #[test]
    fn obnizenie_progu_marginesu_jest_oslabieniem() {
        let mut rachunek = crate::Settings::default();
        rachunek.margin_call_level_pct = 100.0;
        let mut noga = crate::Settings::default();
        noga.margin_call_level_pct = 200.0;
        let nogi: Vec<(&str, &crate::Settings)> = vec![("ZEN", &noga)];
        let v = rozjazd_rachunku(&nogi, &rachunek);
        assert!(v
            .iter()
            .any(|x| x.pole == "margin_call_level_pct" && x.oslabia));

        let mut noga2 = crate::Settings::default();
        noga2.odlicz_kredyt = true;
        let mut rachunek2 = crate::Settings::default();
        rachunek2.odlicz_kredyt = false;
        let nogi2: Vec<(&str, &crate::Settings)> = vec![("ZEN", &noga2)];
        let v2 = rozjazd_rachunku(&nogi2, &rachunek2);
        assert!(v2.iter().any(|x| x.pole == "odlicz_kredyt" && x.oslabia));
    }

    #[test]
    fn zwykly_rozjazd_nie_zatrzymuje_handlu() {
        let rachunek = crate::Settings::default();
        let mut noga = crate::Settings::default();
        noga.commission_per_lot = 7.0; // default 0.0
        noga.exec_latency_ms = 900; // default 250
        let nogi: Vec<(&str, &crate::Settings)> = vec![("Synergy", &noga)];
        let v = rozjazd_rachunku(&nogi, &rachunek);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| !x.oslabia));
    }

    #[test]
    fn bezpieczniki_sa_polami_rachunku() {
        for (k, _) in BEZPIECZNIKI_RACHUNKU {
            assert!(
                crate::wielosilnik::POLA_RACHUNKU.contains(k),
                "`{k}` nie jest polem RACHUNKU — nikt go nogom nie nadpisuje"
            );
        }
    }

    #[test]
    fn pola_parytetu_wykonania_sa_polami_rachunku_i_sa_unikalne() {
        let mut widziane = std::collections::BTreeSet::new();
        for k in POLA_PARYTETU_WYKONANIA {
            assert!(
                crate::wielosilnik::POLA_RACHUNKU.contains(k),
                "`{k}` nie jest polem RACHUNKU — nie może zmienić nogi"
            );
            assert!(widziane.insert(*k), "duplikat pola parytetu `{k}`");
        }
    }

    #[test]
    fn domyslny_zbior_ma_aktywny_ktory_istnieje() {
        let z = Lancuchy::default();
        assert!(z.aktywny().is_some(), "aktywny łańcuch musi być na liście");
        assert_eq!(z.preset_dla("ATFX"), None);
        assert_eq!(z.preset_dla("Synergy"), Some("HYPER-2"));
        assert_eq!(z.preset_dla("ZEN"), Some("FRESHQUEEN-3"));
    }
}
