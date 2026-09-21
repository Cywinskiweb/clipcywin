//! UI strings for the native parts (menus, tray, canvas text). English keys, Polish translations.

use std::sync::atomic::{AtomicU8, Ordering};

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: &str) {
    LANG.store(if lang.eq_ignore_ascii_case("pl") { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn is_pl() -> bool {
    LANG.load(Ordering::Relaxed) == 1
}

/// Translate an English UI string; unknown strings are returned unchanged.
pub fn t(en: &'static str) -> &'static str {
    if !is_pl() {
        return en;
    }
    match en {
        // tray / bar menus
        "Show island" => "Pokaż island",
        "Show bar" => "Pokaż pasek",
        "Mode: Bar" => "Tryb: Pasek",
        "Mode: Island" => "Tryb: Island",
        "Pause capture" => "Wstrzymaj przechwytywanie",
        "Allow screenshots temporarily" => "Zezwól tymczasowo na zrzuty ekranu",
        "Clear history (keep pinned)" => "Wyczyść historię (zostaw przypięte)",
        "Clear everything" => "Wyczyść wszystko",
        "Open shelf" => "Otwórz półkę",
        "Settings…" => "Ustawienia…",
        "Reload settings.json" => "Wczytaj ponownie settings.json",
        "Open data folder" => "Otwórz folder danych",
        "Exit" => "Zakończ",
        // item menu
        "Paste" => "Wklej",
        "Copy to clipboard" => "Kopiuj do schowka",
        "Pin" => "Przypnij",
        "Unpin" => "Odepnij",
        "Mark sensitive" => "Oznacz jako poufne",
        "Unmark sensitive" => "Usuń oznaczenie poufne",
        "Reveal for a moment" => "Pokaż na chwilę",
        "Open image" => "Otwórz obraz",
        "Open containing folder" => "Otwórz folder z plikiem",
        "Add to shelf" => "Dodaj do półki",
        "Delete" => "Usuń",
        // shelf menu
        "Remove from shelf" => "Usuń z półki",
        "Paste all" => "Wklej wszystko",
        "Copy all" => "Kopiuj wszystko",
        "Clear shelf" => "Wyczyść półkę",
        "New shelf" => "Nowa półka",
        "Delete this shelf" => "Usuń tę półkę",
        "Shelf settings…" => "Ustawienia półek…",
        "Shelf" => "Półka",
        // canvas texts
        "Copy something to get started" => "Skopiuj coś, aby zacząć",
        "Drop files, text or images here" => "Upuść tu pliki, tekst lub obrazy",
        "Sensitive" => "Poufne",
        "Image" => "Obraz",
        "File" => "Plik",
        "files" => "plików",
        "chars" => "znaków",
        "lines" => "linii",
        "just now" => "przed chwilą",
        "min ago" => "min temu",
        "h ago" => "godz. temu",
        "d ago" => "dni temu",
        "file(s)" => "plik(ów)",
        "sensitive" => "poufne",
        "alpha" => "alfa",
        "Loading…" => "Wczytywanie…",
        "Sensitive content hidden" => "Poufna treść ukryta",
        "more characters" => "więcej znaków",
        // balloons
        "Copied" => "Skopiowano",
        "Clipboard is busy; try again." => "Schowek jest zajęty, spróbuj ponownie.",
        "Target window is elevated; press Ctrl+V to paste." => "Okno docelowe działa z uprawnieniami administratora; wciśnij Ctrl+V, aby wkleić.",
        "History database unavailable; running without persistence." => "Baza historii niedostępna; działam bez zapisu.",
        "Could not open settings (WebView2 runtime missing?). Edit settings.json in the data folder." => "Nie udało się otworzyć ustawień (brak WebView2?). Edytuj settings.json w folderze danych.",
        "(paused)" => "(wstrzymane)",
        "Clipcywin Settings" => "Clipcywin – Ustawienia",
        _ => en,
    }
}
