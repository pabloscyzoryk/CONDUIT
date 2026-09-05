
use crate::budzet::{BudzetRamy, CelRezerwacji};
use crate::rama::*;
use crate::we::{KoszykDump, OtwarcieZDziennika, SygnalKorpusu, TransakcjaDump};
use std::collections::HashMap;

/// Krok numeracji koszyków w wielosilniku (`wielosilnik.rs:57`).
pub const KROK_SLOTU: u32 = 100_000;

/// POZYCJA — jedna ekspozycja. Transakcje z częściowego zamknięcia noszą ten
/// sam `ticket`, więc sumujemy je z powrotem w jedną pozycję; inaczej ryzyko
/// policzyłoby się tyle razy, ile transz z niej zdjęto.
#[derive(Debug, Clone)]
pub struct Pozycja {
    pub ticket: u64,
    pub koszyk: u32,
    pub strona: Strona,
    pub open_px: f64,
    pub open_ts: Ts,
    pub close_ts: Ts,
    pub wolumen: f64,
    pub netto: f64,
    pub stop_zabral: bool,
    /// poziom siatki z dziennika (`i32::MIN` = nieznany)
    pub poziom: i32,
    /// stop w chwili otwarcia (z dziennika)
    pub sl_przy_otwarciu: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RozliczenieRamy {
    pub rama: u32,
    pub kanal: String,
    pub msg_id: i64,
    pub odcisk: u64,
    pub strona: Strona,
    pub ts_zawiazania: Ts,
    pub ts_konca_ekspozycji: Ts,
    pub etap: EtapRamy,

    pub pl_usd: f64,
    pub budzet_przyznany: f64,
    pub budzet_wydany: f64,
    pub budzet_wykorzystany_pct: Option<f64>,

    /// MFE/MAE RAMY — mierzone tick po ticku na sumie (zrealizowane + otwarte)
    pub mfe_usd: f64,
    pub mae_usd: f64,
    pub ts_mfe: Ts,
    pub ts_mae: Ts,
    pub mfe_silnika_usd: f64,
    /// suma szczytów PER POZYCJA z dziennika — z definicji górne oszacowanie
    pub mfe_suma_pozycji_usd: f64,
    pub zostawione_usd: f64,

    pub prob_lacznie: u32,
    pub pozycji: u32,
    pub miala_pozycje: bool,
    pub stop_zabral: bool,
    /// ile ekspozycji NIE pochodziło ze szczebli planu założycielskiego
    pub pozycji_poza_planem: u32,

    /// GEOMETRIA — do E4
    pub glebokosc_max_w_szer: f64,
    pub fill_ponizej_dalszej_krawedzi: u32,
    pub fillow: u32,
    pub szczebli_planu: u32,

    /// POSŁUSZEŃSTWO / KANAŁ
    pub komend_kanalu: u32,
    pub komend_po_stopie: u32,
    pub komend_zyciowych_po_stopie: u32,

    /// OKNO WAŻNOŚCI POMYSŁU (wypełniane w przebiegu tickowym)
    pub okno: OknoWaznosci,
    /// czy po zabraniu pozycji przez stop pomysł nadal był ważny
    pub wazna_po_stopie_cena: bool,
    pub wazna_po_stopie_silnik: bool,
    pub wazna_po_stopie_kanal: bool,
}

pub struct Portfel {
    pub ramy: Vec<Rama>,
    pub budzety: Vec<BudzetRamy>,
    /// pozycje pogrupowane po ramie (indeks == indeks ramy)
    pub pozycje: Vec<Vec<Pozycja>>,
    /// koszyk → indeks ramy
    pub koszyk_do_ramy: HashMap<u32, usize>,
    /// diagnostyka przypisania
    pub slot_do_kanalu: Vec<(u32, String)>,
    pub koszykow_bez_korpusu: u32,
}

/// Grupuje transakcje z powrotem w POZYCJE (po `ticket`).
pub fn pozycje_z_transakcji(tr: &[TransakcjaDump], otw: &[OtwarcieZDziennika]) -> Vec<Pozycja> {
    let mut z_dziennika: HashMap<u64, &OtwarcieZDziennika> = HashMap::new();
    for o in otw {
        z_dziennika.entry(o.ticket).or_insert(o);
    }
    let mut mapa: HashMap<u64, Pozycja> = HashMap::new();
    for t in tr {
        let Some(k) = t.basket else { continue };
        let e = mapa.entry(t.ticket).or_insert_with(|| Pozycja {
            ticket: t.ticket,
            koszyk: k,
            strona: Strona::z_napisu(&t.side).unwrap_or(Strona::Buy),
            open_px: t.open_price,
            open_ts: t.open_ts,
            close_ts: t.close_ts,
            wolumen: 0.0,
            netto: 0.0,
            stop_zabral: false,
            poziom: i32::MIN,
            sl_przy_otwarciu: None,
        });
        e.wolumen += t.volume;
        e.netto += t.netto();
        e.close_ts = e.close_ts.max(t.close_ts);
        e.stop_zabral |= t.stop_zabral();
    }
    let mut out: Vec<Pozycja> = mapa.into_values().collect();
    for p in out.iter_mut() {
        if let Some(o) = z_dziennika.get(&p.ticket) {
            p.poziom = o.level;
            p.sl_przy_otwarciu = o.sl;
        }
    }
    out.sort_by_key(|p| (p.open_ts, p.ticket));
    out
}

/// Buduje portfel ram. Funkcja jest CZYSTA — dostaje dane, zwraca strukturę.
pub fn zbuduj(
    koszyki: &[KoszykDump],
    transakcje: &[TransakcjaDump],
    korpus: &[SygnalKorpusu],
    otwarcia: &[OtwarcieZDziennika],
    krok_odcisku: f64,
) -> Portfel {
    // --- 1. slot → kanał (głosowanie po msg_id, bo identyfikatory Telegrama
    //        są PER CZAT i te same liczby wracają w różnych kanałach) ---
    let mut po_id: HashMap<i64, Vec<&SygnalKorpusu>> = HashMap::new();
    for s in korpus {
        po_id.entry(s.id).or_default().push(s);
    }
    let mut glosy: HashMap<u32, HashMap<String, u32>> = HashMap::new();
    for k in koszyki {
        let slot = k.id / KROK_SLOTU;
        if let Some(v) = po_id.get(&k.msg_id) {
            if v.len() == 1 {
                *glosy
                    .entry(slot)
                    .or_default()
                    .entry(v[0].kanal.clone())
                    .or_insert(0) += 1;
            }
        }
    }
    let mut slot_do_kanalu: Vec<(u32, String)> = glosy
        .into_iter()
        .map(|(slot, m)| {
            let mut v: Vec<(String, u32)> = m.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            (slot, v[0].0.clone())
        })
        .collect();
    slot_do_kanalu.sort();
    let kanal_slotu: HashMap<u32, String> = slot_do_kanalu.iter().cloned().collect();

    // --- 2. pozycje ---
    let wszystkie_pozycje = pozycje_z_transakcji(transakcje, otwarcia);
    let mut pozycje_koszyka: HashMap<u32, Vec<Pozycja>> = HashMap::new();
    for p in wszystkie_pozycje {
        pozycje_koszyka.entry(p.koszyk).or_default().push(p);
    }

    // --- 3. ramy ---
    let mut ramy: Vec<Rama> = Vec::new();
    let mut budzety: Vec<BudzetRamy> = Vec::new();
    let mut pozycje: Vec<Vec<Pozycja>> = Vec::new();
    let mut klucz_do_ramy: HashMap<(u32, i64), usize> = HashMap::new();
    let mut koszyk_do_ramy: HashMap<u32, usize> = HashMap::new();
    let mut bez_korpusu = 0u32;

    let mut lista: Vec<&KoszykDump> = koszyki.iter().collect();
    lista.sort_by_key(|k| (k.created_ts, k.id));

    for k in lista {
        let slot = k.id / KROK_SLOTU;
        let kanal = kanal_slotu
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| format!("slot{slot}"));
        let sygnal = po_id.get(&k.msg_id).and_then(|v| {
            v.iter()
                .find(|s| s.kanal == kanal)
                .or_else(|| v.first())
                .copied()
        });
        if sygnal.is_none() {
            bez_korpusu += 1;
        }
        let strona = match &sygnal {
            Some(s) => Strona::z_napisu(&s.dir),
            None => Strona::z_napisu(&k.side),
        }
        .unwrap_or(Strona::Buy);

        let geo = match &sygnal {
            Some(s) => GeometriaPomyslu::nowa(
                strona,
                s.lo,
                s.hi,
                if s.sl != 0.0 { Some(s.sl) } else { None },
                s.tps.clone(),
            ),
            None => GeometriaPomyslu::nowa(
                strona,
                if k.entry_lo != 0.0 {
                    k.entry_lo
                } else {
                    k.zone_lo
                },
                if k.entry_hi != 0.0 {
                    k.entry_hi
                } else {
                    k.zone_hi
                },
                k.sl,
                k.tps.clone(),
            ),
        };
        let ts_pomyslu = match &sygnal {
            Some(s) => s.ts * 1000,
            None => k.created_ts,
        };

        let klucz = (slot, k.msg_id);
        let idx = match klucz_do_ramy.get(&klucz) {
            Some(i) => *i,
            None => {
                let id = ramy.len() as u32 + 1;
                let r = Rama::zawiaz(
                    id,
                    kanal.clone(),
                    k.msg_id,
                    geo,
                    k.id,
                    ts_pomyslu,
                    krok_odcisku,
                );
                // BUDŻET PRZYZNANY = ryzyko PEŁNEGO planu założycielskiego
                // wobec stopu POMYSŁU. To jest dokładnie ta wielkość, którą
                // dzisiejszy `cap_basket_risk` liczy dla JEDNEGO planu.
                let przyznany: f64 = k
                    .warstwy
                    .iter()
                    .filter_map(|w| r.geometria.ryzyko_nogi(w.cena_zlecenia, w.wolumen))
                    .sum();
                budzety.push(BudzetRamy::ksiega_bez_sufitu(id, przyznany));
                ramy.push(r);
                pozycje.push(Vec::new());
                klucz_do_ramy.insert(klucz, ramy.len() - 1);
                ramy.len() - 1
            }
        };
        koszyk_do_ramy.insert(k.id, idx);

        // --- próba ---
        let poz = pozycje_koszyka.remove(&k.id).unwrap_or_default();
        let nr = ramy[idx].proby.len() as u32 + 1;
        let ryzyko_planu: f64 = k
            .warstwy
            .iter()
            .filter_map(|w| ramy[idx].geometria.ryzyko_nogi(w.cena_zlecenia, w.wolumen))
            .sum();
        let proba = Proba {
            nr,
            koszyk: k.id,
            rola: if nr == 1 {
                RolaProby::Baza
            } else {
                RolaProby::Powrot
            },
            strona,
            ryzyko_rezerwacji: ryzyko_planu,
            ts_otwarcia: k.first_open_ts,
            ts_zamkniecia: k.last_close_ts,
            wynik_usd: poz.iter().map(|p| p.netto).sum(),
            miala_pozycje: k.had_positions || !poz.is_empty(),
            stop_zabral: poz.iter().any(|p| p.stop_zabral),
        };
        ramy[idx].proby.push(proba);

        // --- etapy (monotoniczne) ---
        if !k.warstwy.is_empty() {
            ramy[idx].awansuj(EtapRamy::Czuwa, k.created_ts);
        }
        if k.had_positions || !poz.is_empty() {
            ramy[idx].awansuj(EtapRamy::Zaangazowana, k.first_open_ts.max(k.created_ts));
        }
        if k.secured {
            ramy[idx].awansuj(EtapRamy::Zabezpieczona, k.last_close_ts.max(k.created_ts));
        }
        if k.state == "Done" {
            ramy[idx].awansuj(EtapRamy::Zamknieta, k.last_close_ts.max(k.created_ts));
        }

        // --- KSIĘGA BUDŻETU: każda ekspozycja przechodzi przez kwit ---
        for p in &poz {
            let cel = if p.poziom >= 0 && nr == 1 {
                CelRezerwacji::SiatkaPoczatkowa
            } else if p.poziom == i32::MIN {
                CelRezerwacji::NieprzypisanaEkspozycja
            } else {
                CelRezerwacji::DokladkaRynkowa
            };
            // ryzyko liczone wobec stopu POMYSŁU (jednostka wspólna dla całej
            // ramy); stop z chwili otwarcia jest drugim świadkiem i wchodzi
            // do raportu osobno
            let ryz = ramy[idx]
                .geometria
                .ryzyko_nogi(p.open_px, p.wolumen)
                .unwrap_or(0.0);
            // margines: 0,01 lota XAUUSD przy 1:500 i cenie ~4630 to 9,26 $,
            // czyli px × 100 × wolumen / dźwignia
            let margines = p.open_px * XAU_CONTRACT * p.wolumen / 500.0;
            if let Ok(kwit) = budzety[idx].rezerwuj(nr, ryz, margines, cel) {
                budzety[idx].zuzyj(kwit, ryz, margines);
            }
            budzety[idx].rozlicz_zamkniecie(ryz, p.netto);
        }
        pozycje[idx].extend(poz);
    }

    Portfel {
        ramy,
        budzety,
        pozycje,
        koszyk_do_ramy,
        slot_do_kanalu,
        koszykow_bez_korpusu: bez_korpusu,
    }
}

#[cfg(test)]
mod testy {
    use super::*;
    use crate::we::*;

    fn kosz(id: u32, msg: i64, ceny: &[f64]) -> KoszykDump {
        KoszykDump {
            id,
            msg_id: msg,
            side: "Buy".into(),
            zone_lo: 4100.0,
            zone_hi: 4106.0,
            sl: Some(4094.0),
            tps: vec![4110.0],
            created_ts: 1_000,
            tp_stage: 0,
            tp_touch_ts: vec![],
            sl_touch_ts: 0,
            warstwy: ceny
                .iter()
                .enumerate()
                .map(|(i, c)| WarstwaDump {
                    poziom: i as i32,
                    cena_zlecenia: *c,
                    wolumen: 0.01,
                    fill_ts: 0,
                    fill_px: 0.0,
                    anulowana: false,
                    toucher: false,
                    filled: false,
                })
                .collect(),
            pl: 0.0,
            n_trades: 0,
            first_open_ts: 0,
            last_close_ts: 0,
            reentries: 0,
            rearms: 0,
            had_positions: false,
            secured: false,
            peak_pl_usd: 0.0,
            entry_lo: 4100.0,
            entry_hi: 4106.0,
            state: "Done".into(),
        }
    }

    fn trans(t: u64, b: u32, px: f64, vol: f64, netto: f64, powod: &str) -> TransakcjaDump {
        TransakcjaDump {
            ticket: t,
            side: "Buy".into(),
            volume: vol,
            open_price: px,
            close_price: px,
            open_ts: 2_000,
            close_ts: 3_000,
            profit: netto,
            validated_net: netto,
            commission: 0.0,
            swap: 0.0,
            reason: powod.into(),
            basket: Some(b),
        }
    }

    fn sygnal(id: i64, kanal: &str) -> SygnalKorpusu {
        SygnalKorpusu {
            id,
            ts: 1,
            dir: "BUY".into(),
            limit: true,
            lo: 4100.0,
            hi: 4106.0,
            sl: 4094.0,
            tps: vec![4110.0],
            kanal: kanal.into(),
            text: String::new(),
            events: vec![],
        }
    }

    /// Częściowe zamknięcia MUSZĄ wrócić do jednej pozycji — inaczej ryzyko
    /// policzyłoby się dwa razy i `budzet_wykorzystany_pct` byłby zmyślony.
    #[test]
    fn transze_wracaja_do_jednej_pozycji() {
        let tr = vec![
            trans(9, 1, 4104.0, 0.01, 1.0, "Tp"),
            trans(9, 1, 4104.0, 0.01, 2.0, "Trail"),
        ];
        let p = pozycje_z_transakcji(&tr, &[]);
        assert_eq!(p.len(), 1);
        assert!((p[0].wolumen - 0.02).abs() < 1e-12);
        assert!((p[0].netto - 3.0).abs() < 1e-12);
    }

    #[test]
    fn budzet_liczy_cale_zycie_ramy_a_przyznany_tylko_plan() {
        let koszyki = vec![kosz(1, 77, &[4104.0, 4102.0])];
        // dwie nogi z planu + trzecia spoza planu (dokładka)
        let tr = vec![
            trans(1, 1, 4104.0, 0.01, -1.0, "Sl"),
            trans(2, 1, 4102.0, 0.01, -1.0, "Sl"),
            trans(3, 1, 4103.0, 0.01, -1.0, "Sl"),
        ];
        let korpus = vec![sygnal(77, "SyntheticFormatA")];
        let p = zbuduj(&koszyki, &tr, &korpus, &[], 1.0);
        assert_eq!(p.ramy.len(), 1);
        // przyznany = (4104-4094)*100*0.01 + (4102-4094)*100*0.01 = 10 + 8 = 18
        assert!((p.budzety[0].przyznany() - 18.0).abs() < 1e-9);
        // wydany = 10 + 8 + 9 = 27  →  150 %
        assert!((p.budzety[0].wydany() - 27.0).abs() < 1e-9);
        assert!((p.budzety[0].wykorzystany_pct().unwrap() - 150.0).abs() < 1e-9);
        assert!(p.ramy[0].stop_zabral());
    }

    /// Kanał czytamy ze SLOTU, nie z `msg_id` — identyfikatory Telegrama
    /// kolidują między czatami.
    #[test]
    fn slot_rozstrzyga_kanal_przy_kolizji_identyfikatorow() {
        let mut a = kosz(3, 77, &[4104.0]);
        a.id = 1 * KROK_SLOTU + 3;
        let mut b = kosz(4, 77, &[4104.0]);
        b.id = 2 * KROK_SLOTU + 4;
        // Unikalne kotwice pozwalają najpierw uczciwie zidentyfikować kanał
        // każdego slotu. Sam msg_id=77 celowo występuje potem w OBU kanałach,
        // więc nie może zagłosować i musi zostać rozstrzygnięty slotem.
        let mut kotwica_a = kosz(1, 101, &[4104.0]);
        kotwica_a.id = 1 * KROK_SLOTU + 1;
        let mut kotwica_b = kosz(2, 202, &[4104.0]);
        kotwica_b.id = 2 * KROK_SLOTU + 2;
        let korpus = vec![
            sygnal(101, "SyntheticFormatA"),
            sygnal(202, "SyntheticFormatB"),
            sygnal(77, "SyntheticFormatA"),
            sygnal(77, "SyntheticFormatB"),
        ];
        let p = zbuduj(&[kotwica_a, kotwica_b, a, b], &[], &korpus, &[], 1.0);
        let kolizyjne: Vec<_> = p.ramy.iter().filter(|r| r.msg_id == 77).collect();
        assert_eq!(
            kolizyjne.len(),
            2,
            "dwa sloty = dwie różne ramy mimo tego samego msg_id"
        );
        assert_ne!(kolizyjne[0].kanal, kolizyjne[1].kanal);
        assert_eq!(
            kolizyjne.iter().map(|r| r.kanal.as_str()).collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["SyntheticFormatA", "SyntheticFormatB"]),
        );
    }
}
