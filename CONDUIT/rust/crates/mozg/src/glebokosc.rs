
use crate::rama::GeometriaPomyslu;

/// Jedna noga planu założycielskiego, przypisana do szczebla.
#[derive(Debug, Clone)]
pub struct NogaPlanu {
    pub rama: usize,
    pub poziom: i32,
    /// głębokość CENY ZLECENIA w szerokościach strefy pomysłu
    pub glebokosc_zlecenia: f64,
    /// głębokość CENY WYPEŁNIENIA (przy luce potrafi być głębsza)
    pub glebokosc_fillu: Option<f64>,
    pub wolumen: f64,
    pub wypelniona: bool,
    /// wynik NETTO wszystkich transakcji tej nogi ($)
    pub netto: f64,
    /// czy wyjście tej nogi zapadło REGUŁĄ KOSZYKOWĄ (łamie addytywność)
    pub wyjscie_koszykowe: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WierszKontrfaktyku {
    pub prog_d: f64,
    pub szczebli_planu: u32,
    pub szczebli_zostaje: u32,
    pub fillow: u32,
    pub fillow_zostaje: u32,
    pub wolumen: f64,
    pub wolumen_zostaje: f64,
    pub wynik_usd: f64,
    pub wynik_zostaje_usd: f64,
    /// ile ram straciłoby WSZYSTKIE swoje wypełnienia (czyli nie weszłoby wcale)
    pub ram_bez_wejscia: u32,
    pub ram_z_fillami: u32,
    /// ile z odciętych nóg wychodziło regułą koszykową (miara kłamstwa założenia)
    pub odcietych_z_wyjsciem_koszykowym: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RozkladGlebokosci {
    pub fillow: u32,
    pub ponizej_dalszej_krawedzi: u32,
    pub mediana: f64,
    pub p90: f64,
    pub max: f64,
}

pub fn rozklad_glebokosci(nogi: &[NogaPlanu]) -> RozkladGlebokosci {
    let mut g: Vec<f64> = nogi
        .iter()
        .filter(|n| n.wypelniona)
        .filter_map(|n| n.glebokosc_fillu.or(Some(n.glebokosc_zlecenia)))
        .collect();
    if g.is_empty() {
        return RozkladGlebokosci::default();
    }
    g.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = g.len();
    RozkladGlebokosci {
        fillow: n as u32,
        ponizej_dalszej_krawedzi: g.iter().filter(|x| **x > 1.0).count() as u32,
        mediana: g[n / 2],
        p90: g[(n as f64 * 0.9) as usize % n],
        max: g[n - 1],
    }
}

/// Liczy jeden wiersz kontrfaktyku dla progu `d`.
///
/// Szczebel przeżywa, gdy głębokość jego CENY ZLECENIA ≤ `d`. Nie ceny
/// wypełnienia — bo decyzja o płytszej siatce zapada przy PLANOWANIU,
/// a nie po fakcie. To rozróżnienie kosztuje kilka procent liczby i jest
/// jedyną wersją, która nie zagląda w przyszłość.
pub fn wiersz(nogi: &[NogaPlanu], n_ram: usize, d: f64) -> WierszKontrfaktyku {
    let mut w = WierszKontrfaktyku {
        prog_d: d,
        ..Default::default()
    };
    let mut fill_ram = vec![0u32; n_ram];
    let mut fill_ram_zostaje = vec![0u32; n_ram];
    for n in nogi {
        w.szczebli_planu += 1;
        let zostaje = n.glebokosc_zlecenia <= d + 1e-9;
        if zostaje {
            w.szczebli_zostaje += 1;
        }
        if n.wypelniona {
            w.fillow += 1;
            w.wolumen += n.wolumen;
            w.wynik_usd += n.netto;
            fill_ram[n.rama] += 1;
            if zostaje {
                w.fillow_zostaje += 1;
                w.wolumen_zostaje += n.wolumen;
                w.wynik_zostaje_usd += n.netto;
                fill_ram_zostaje[n.rama] += 1;
            } else if n.wyjscie_koszykowe {
                w.odcietych_z_wyjsciem_koszykowym += 1;
            }
        }
    }
    for i in 0..n_ram {
        if fill_ram[i] > 0 {
            w.ram_z_fillami += 1;
            if fill_ram_zostaje[i] == 0 {
                w.ram_bez_wejscia += 1;
            }
        }
    }
    w
}

/// Cała krzywa — to jest wynik, nie pojedynczy próg.
pub fn krzywa(nogi: &[NogaPlanu], n_ram: usize, progi: &[f64]) -> Vec<WierszKontrfaktyku> {
    progi.iter().map(|d| wiersz(nogi, n_ram, *d)).collect()
}

/// Głębokość ceny wobec strefy POMYSŁU (nie strefy po offsetach silnika).
#[inline]
pub fn glebokosc(g: &GeometriaPomyslu, px: f64) -> f64 {
    g.glebokosc(px)
}

#[cfg(test)]
mod testy {
    use super::*;

    fn noga(rama: usize, d: f64, wypelniona: bool, netto: f64) -> NogaPlanu {
        NogaPlanu {
            rama,
            poziom: 0,
            glebokosc_zlecenia: d,
            glebokosc_fillu: Some(d),
            wolumen: 0.01,
            wypelniona,
            netto,
            wyjscie_koszykowe: false,
        }
    }

    #[test]
    fn plytsza_siatka_odcina_glebokie_nogi_i_ich_wynik() {
        let nogi = vec![
            noga(0, 0.0, true, -1.0),
            noga(0, 0.5, true, 2.0),
            noga(0, 1.2, true, 10.0), // za dalszą krawędzią
        ];
        let w = wiersz(&nogi, 1, 1.0);
        assert_eq!(w.fillow, 3);
        assert_eq!(w.fillow_zostaje, 2);
        assert!((w.wynik_usd - 11.0).abs() < 1e-9);
        assert!((w.wynik_zostaje_usd - 1.0).abs() < 1e-9);
        assert_eq!(w.ram_bez_wejscia, 0);
    }

    #[test]
    fn prog_zero_zostawia_tylko_krawedz_i_wywala_ramy_bez_wejscia() {
        let nogi = vec![noga(0, 0.4, true, 5.0), noga(1, 0.0, true, -2.0)];
        let w = wiersz(&nogi, 2, 0.0);
        assert_eq!(w.fillow_zostaje, 1);
        assert_eq!(w.ram_z_fillami, 2);
        assert_eq!(w.ram_bez_wejscia, 1, "rama 0 nie weszłaby wcale");
    }

    #[test]
    fn rozklad_liczy_udzial_ponizej_dalszej_krawedzi() {
        let nogi = vec![
            noga(0, 0.2, true, 0.0),
            noga(0, 1.1, true, 0.0),
            noga(0, 1.5, true, 0.0),
            noga(0, 0.9, false, 0.0), // niewypełniona nie liczy się do rozkładu
        ];
        let r = rozklad_glebokosci(&nogi);
        assert_eq!(r.fillow, 3);
        assert_eq!(r.ponizej_dalszej_krawedzi, 2);
        assert!((r.max - 1.5).abs() < 1e-9);
    }
}
