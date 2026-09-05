# CONDUIT — wydanie z GOD-X7

GOD-X7 pozostaje głównym presetem zgodnie z wyborem właściciela. Nie nadano tytułu GOD-X8 innemu kandydatowi. Wyniki badań pozostają opisem wskazanych danych historycznych; wydanie nie deklaruje określonego odsetka dodatnich dni ani przyszłego zysku.

## Co się zmieniło

- **Zgodność wykonania i odtwarzania.** Poprawiono obsługę potwierdzeń operacji brokerowych, odtwarzanie stanu oraz przypadki częściowych zamknięć i dodatkowych wejść. Lokalna odmowa FastAddon z powodu TP zachowuje wolne miejsce i respektuje cooldown presetu. Nowe zabezpieczenia nie stanowią obietnicy identycznych cen wykonania u różnych brokerów.
- **Telegram i edycje.** Doprecyzowano powiązanie NEW, EDIT i anulowania z właściwym źródłem. Obsługa pierwszego kompletnego sygnału widzianego jako edycja zależy od ustawienia presetu i działa w chwili odbioru. Ważność jawnych LIMIT/STOP oraz zarządzanie oczekującymi zleceniami pozostają oddzielnymi ustawieniami. Historyczny eksport zawierający tylko końcową wersję wiadomości nie jest przedstawiany jako pełny zapis jej wcześniejszych wersji.
- **Czas.** Kwotowania i historia brokera zachowują czas serwera; frontend nie dodaje do niego ponownie lokalnej strefy przeglądarki. Wiadomości Telegrama mają odrębną domenę UTC. Wiek kwotowania i filtrowanie historii korzystają z jawnych źródeł czasu, a brak danych nie jest przedstawiany jako świeże kwotowanie.
- **Wynik i koszty.** Historia, eksporty i prezentacja poczty korzystają z jawnej podstawy wyniku, aby nie doliczać swapu drugi raz. Wynik zarządzania strategii jest podpisany oddzielnie od rozliczonego wyniku netto. Brak potwierdzonej podstawy lub kosztów nie jest zamieniany na pozornie dokładne zero.
- **Interfejs i języki.** Zachowano gęstość informacji i układ zbliżony do wcześniejszego panelu, poprawiając czytelność i dostęp do rozbudowanych ustawień. Uzupełniono angielski w polach, opcjach, komunikatach i generowanej poczcie. Postęp prac ma własny wybór PL/EN, etap, licznik wykonania, tempo oraz jawny tryb obliczeń. Ograniczono powtarzające się opisy bez ukrywania istotnych danych.
- **Uruchamianie i pakowanie.** Dodano przenośne środowisko Python i aplikację postępu do paczki oraz awaryjne otwarcie panelu w przeglądarce, jeśli okno WebView2 nie może wystartować. Weryfikacja paczki pilnuje zgodności wybranego presetu, limitów łańcucha, wieku odbieranych sygnałów i ustawień rachunku. Techniczna ścieżka terminala może być inna; nakładka nie może po cichu zmienić podstawy lota, kosztów, dźwigni ani zarządzania AI.

## Zasady obu paczek

PUBLIC ma pozostać bez danych logowania, sesji Telegrama i prywatnych przypisań kanałów; uruchamia się jako nieskonfigurowana wersja MANUAL. Paczka VPS zachowuje dane i sposób wyboru konta z wskazanego prywatnego szablonu, w tym FOLLOW_TERMINAL. Strategia i profil ekonomiczny pochodzą z zaakceptowanego, przetestowanego scenariusza, a nie ze starej konfiguracji szablonu.

Nowe osie badań nie są automatycznie włączane tylko dlatego, że istnieją w programie. Ich stan określa wybrany preset i jawny kontrakt rachunku. Prywatne sesje, archiwa oraz dane wejściowe badań nie należą do publicznego kodu ani wydania.

## Zakres potwierdzenia

Opis dotyczy ukończonych zmian kodu i narzędzi. Końcowy manifest wydania powinien wskazać dokładny build, preset i testy gotowej paczki. Testy offline i poprawny manifest nie oznaczają sprawdzenia logowania do usług ani przyszłych wyników. Nie wykonywano wysyłki próbnej do użytkowników w ramach testów renderowania poczty.

## English release summary

GOD-X7 remains the primary preset by the owner's choice. This release improves execution acknowledgements, restart recovery, Telegram edit handling, clock domains, explicit profit accounting, English translations and portable packaging. Strategy results are distinguished from confirmed net account results. The public package contains no private credentials or Telegram sessions; the VPS package preserves the selected private account configuration. New research settings are not enabled automatically. Historical results are not a forecast or a guarantee of profitable trading days.
