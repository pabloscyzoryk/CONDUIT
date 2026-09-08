# Odtwarzanie zarejestrowanej sesji live

Nagranie służy do sprawdzenia, czy silnik podejmuje te same decyzje przy tych
samych zaobserwowanych wejściach i odpowiedziach brokera. Jest częścią
diagnostyki. Nie jest nowym presetem ani symulacją hipotetycznej realizacji
zleceń.

Kronika Telegrama zachowuje wersje wiadomości i ich odbiór. Dodatkowe nagranie
live zachowuje stan decyzyjny silnika, kolejność wywołań i odczytane dane
brokera. Obejmuje również błędy i oczekiwanie na potwierdzenie: późniejsze
wykrycie pozycji nie jest zamieniane na natychmiastowy poprawny ACK.

## Eksport

W „Logi i raporty → Scalanie” źródło **nagranie odtwarzania live** jest
domyślnie włączone, także po aktualizacji starszych ustawień. `alllogs` dołącza
manifesty i treść całych zachowanych sesji do ich ostatniego opublikowanego
numeru zdarzenia. Nie sortuje ponownie ani nie filtruje ich pojedynczych
wierszy według daty. Początek sesji jest potrzebny także wtedy, gdy analizowane
zdarzenie wystąpiło później.

Zachowaj oryginalny plik `alllogs` oraz eksport ticków. Nie poprawiaj ręcznie
czasów ani tekstu. Granice nagrania, skróty SHA256 i liczby bajtów pozwalają
wykryć uszkodzenie lub obcięcie danych. Surowe pliki nagrania są w
`logs/replay_capture/` obok danych instalacji. Usunięcie starej sesji przez
retencję ma osobny zapis; fragment z brakującym początkiem nie otrzymuje
statusu pełnego odtworzenia.

## Sprawdzenie offline

W PowerShell, w folderze wydania:

```powershell
.\conduit.exe --replay-capture "C:\diagnostyka\alllogs.txt" --replay-report "C:\diagnostyka\wynik-replay.json"
```

Tryb ten kończy się przed uruchomieniem zwykłej aplikacji, jej okna i usług.
Nie łączy się z MT5 ani Telegramem. Wskaż nową nazwę raportu: istniejący
wynik nie jest nadpisywany ani ponownie uznawany za wynik bieżącego testu.
`PASS` potwierdza wyłącznie opisany w raporcie zakres; błąd lub brak danych
oznacza wynik niezerowy. Raport zawiera tożsamość nagrania i odtwarzacza.

Z pliku allLogs odtwarzacz wyodrębnia prywatne sesje obok raportu, w folderze
z rozszerzeniem `capture-<PID>`. Sprawdzanie zatrzymuje się przy pierwszym
niepoprawnym nagraniu. Zachowane kompletne sesje można wtedy sprawdzić
osobno, podając katalog konkretnej sesji zamiast pliku allLogs. Żadna sesja
z luką nie jest po cichu pomijana przy wystawianiu wyniku całego eksportu.

Można też jawnie wybrać jedną sesję bezpośrednio z allLogs:

```powershell
.\conduit.exe --replay-capture "C:\diagnostyka\alllogs.txt" --replay-run "dokladny-identyfikator-sesji" --replay-report "C:\diagnostyka\wynik-wybranej-sesji.json"
```

Identyfikator odczytaj z manifestu nagrania. Raport zaznacza wybór i pominięte
sesje; wynik dotyczy wyłącznie wskazanego odcinka. Uszkodzenie wybranej sesji
nadal powoduje błąd. Opcja nie jest sposobem na uznanie całego eksportu za
zgodny mimo wcześniejszej luki.

## Znaczenie zgodności

- Początkowy stan obejmuje także pamięć sygnałów, cooldowny, odroczone
  operacje, historię wskaźników i dotychczasowe koszyki.
- Czas odbioru Telegrama, czas kwotowania brokera i kolejność wykonania są
  różnymi informacjami. Edycja obowiązuje od rzeczywistego odbioru.
- Nagranie zachowuje dokładną reprezentację liczb zmiennoprzecinkowych.
  Zapis JSON nie zaokrągla parametrów decyzji.
- Odtwarzacz sprawdza parametry i kolejność odczytów oraz operacji, a także
  stan po zdarzeniu. Pierwsza rozbieżność kończy weryfikację.
- Wersja kontraktu obejmuje silnik reguł, w tym GOD-X7, z wyłączonym AI i EA.
  Pozostałe tryby wymagają osobnego kontraktu stanu i nie otrzymują tego
  samego potwierdzenia.

Potwierdzenie dotyczy konkretnego nagranego zakresu oraz zgodności silnika
dla zarejestrowanych wejść. Odbiór i filtrowanie Telegrama sprawdza się osobno
na podstawie kroniki; replay zaakceptowanych wejść sam nie wykryje wiadomości,
która nigdy nie dotarła do silnika.
Nie dowodzi, że broker wykonał wszystkie czynności idealnie, ani że strategia
jest zyskowna. Odtwarzacz celowo otrzymuje takie odpowiedzi, jakie zobaczył
bot, również niekorzystne lub niejednoznaczne. Eksport ticków pozwala
niezależnie sprawdzić ceny i momenty, lecz nie odtworzy nieznanych odpowiedzi
MT5 lub stanu sprzed rozpoczęcia zapisu.

## Granice i retencja

Nagranie jest dzielone między decyzjami po osiągnięciu 64 MiB. Kolejna sesja
otrzymuje pełny aktualny stan silnika i identyfikator poprzedniej. Ręczna
komenda z panelu również kończy poprzedni odcinek i rozpoczyna nowy od stanu
po ingerencji. Sama polityka wykonania tej komendy pozostaje poza kontraktem
replay; granica jest jawna. Nie zmienia to stanu strategii ani kolejności
jej zdarzeń.

Zapis ma twardy budżet 512 MiB na sesję i łącznie. Retencja usuwa całe stare,
zamknięte nagrania, zachowując informację o usunięciu. Nie usuwa początku
aktywnej sesji, aby udawać kompletną historię. Kolejka ma limit 16 MiB,
a pojedynczy rekord 8 MiB. Przepełnienie lub błąd zapisu daje jawną lukę
i ostrzeżenie. Po rzeczywistej luce nagrywanie nie odnawia się automatycznie
z domniemaniem ciągłości.

Kompresja i współdzielenie powtarzających się wartości są bezstratne. Każdy
odczyt brokera nadal następuje; odwołanie do wcześniejszej wartości jest
używane dopiero po dokładnym porównaniu. Eksport aktywnej sesji obejmuje
ostatnią opublikowaną, kompletną decyzję. Nie obejmuje niezatwierdzonego
jeszcze końca kolejki. Granicę wyznacza numer ostatniego zdarzenia w manifeście.

Odtworzenie nagrania i backtest są odrębnymi kontrolami. Oba wykonują tę samą
logikę `Engine`. W replay broker zwraca faktycznie zaobserwowane odpowiedzi;
w backteście model wykonania wyznacza je z danych rynku i jawnych parametrów.
Zgodność replay nie potwierdza sama w sobie zgodności modelu brokera. Rozjazd
trzeba znaleźć na pierwszej różnej decyzji albo odpowiedzi i naprawić ogólną
regułę. Nie koryguje się wyników pod konkretny dzień, wiadomość ani transakcję.

## Prywatność

Nagranie jest prywatnym materiałem diagnostycznym. Może zawierać treść
wiadomości, identyfikatory źródeł i transakcji, tożsamość rachunku, ustawienia
oraz wyniki. Hasła, klucze API i klucze sesji uwierzytelniającej nie są danymi
kontraktu nagrania. Plików `logs/replay_capture`, kroniki ani `alllogs` nie
dołącza się do publicznego repozytorium lub publicznej paczki wydania.

## English

Live capture records the rules engine's decision state, ordered inputs and
the broker responses actually observed by the bot. It complements the
Telegram Chronicle and tick exports. The default allLogs export includes
complete retained recording prefixes, their manifests and integrity checks.
It never silently reconstructs missing session origins or acknowledgements.

The verifier compares call order, arguments and resulting engine state,
including exact floating-point representations. Its current qualification
covers the rules engine with AI and EA disabled, including GOD-X7. A match
describes the recorded interval and engine behaviour, not ideal broker
execution or future profitability. Capture files contain private message,
account and transaction information and must remain outside public releases.

Replay and backtesting are separate checks using the same Engine logic.
Replay supplies observed broker responses; backtesting derives responses from
market data and an explicit execution model. A replay match alone does not
validate that execution model. Fixes must address the first divergent general
rule, without signal-specific, date-specific or trade-specific adjustments.
