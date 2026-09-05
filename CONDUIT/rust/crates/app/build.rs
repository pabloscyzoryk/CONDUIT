fn main() {
    // Tauri generuje kontekst (konfiguracja, uprawnienia, zasoby Windows).
    // Bez cechy `window` binarka jest czysto serwerowa i nic tu nie robimy.
    #[cfg(feature = "window")]
    tauri_build::build();
}
