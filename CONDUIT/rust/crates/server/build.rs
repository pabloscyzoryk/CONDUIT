
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    obserwuj(Path::new("web"));
}

/// Zgłasza Cargo katalog i wszystko, co w nim leży, jako zależność budowy.
///
/// Zgłaszamy KATALOGI (żeby złapać dopisanie i usunięcie pliku) ORAZ pliki
/// (żeby złapać podmianę treści przy niezmienionej liście). Brak katalogu nie
/// jest błędem: `web/` powstaje dopiero po pierwszym `npm run build`, a serwer
/// musi dać się zbudować także bez panelu — wtedy `rust_embed` wkompilowuje
/// pustkę, a `WebSource::Disk` pozwala podać pliki z dysku.
fn obserwuj(dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(wpisy) = std::fs::read_dir(dir) else {
        return;
    };
    for wpis in wpisy.flatten() {
        let sciezka = wpis.path();
        if sciezka.is_dir() {
            obserwuj(&sciezka);
        } else {
            println!("cargo:rerun-if-changed={}", sciezka.display());
        }
    }
}
