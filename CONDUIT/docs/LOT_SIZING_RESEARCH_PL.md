# Badanie lotowania GOD-X7

Status: warstwa eksperymentalna. GOD-X7 pozostaje domyślny. Wynik badania nie
stanowi automatycznego zatwierdzenia GOD-X8 ani aktualizacji działającego VPS.
Kwalifikacja mostka dotyczy konfiguracji GOD-X7 z włączonym
`close_receipt_reconcile`. Wyłączenie tego mechanizmu potwierdzania operacji
brokera nie jest objęte kwalifikacją nowych ustawień lotowania.

Badanie zachowuje parser, źródła, kierunek, strefę wejścia, SL, TP, rearm i
dotychczasowe reguły GOD-X7. Nowe ustawienia ustalają wolumen osobnej pozycji.
Wolumen może pośrednio zmienić dalszą ścieżkę strategii, ponieważ wpływa na
equity, margin i dostępność kolejnych zleceń. Nie zakładamy identycznej liczby
transakcji po zmianie wielkości pozycji.

## Krzywa kapitału i podział między pozycje

Nominalny lot może rosnąć według potęgi salda, liniowo po przekroczeniu progu
lub schodkami geometrycznymi. Następnie podział między pozycje uwzględnia
odległość konkretnego wejścia od SL, położenie w strefie albo już otwarte
ryzyko koszyka. Dostępny jest również podział równy.

## Dziesięć dodatkowych osi

| Oś | Obserwacja dostępna przed wysłaniem zlecenia | Kierunek zmiany |
|---|---|---|
| Equity względem balance | Niezrealizowana strata / balance | Mniejszy przyrost lota przy większej stracie |
| Ryzyko portfela | Łączne reprezentowane ryzyko do SL / equity | Mniejszy przyrost przy większym obciążeniu |
| Ryzyko kierunku | Ryzyko BUY lub SELL zgodne z nową pozycją / equity | Mniejszy przyrost przy większej koncentracji |
| Liczba koszyków | Liczba koszyków z pozycjami lub zleceniami | Mniejszy przyrost przy wielu aktywnych koszykach |
| Spread | Spread względem odległości nowego wejścia od SL | Mniejszy przyrost przy relatywnie wysokim koszcie |
| Relacja TP1 do SL | Pozostała droga do TP1 względem drogi do SL | Mniejszy przyrost przy słabej relacji |
| Szerokość stopa | Droga do SL względem szerokości zaakceptowanej strefy | Mniejszy przyrost przy relatywnie szerokim stopie |
| Wiek koszyka | Czas od faktycznego utworzenia koszyka | Mniejszy przyrost dla starszego koszyka; bez anulowania sygnału |
| Liczba rearmów | Liczba już wykonanych ponownych uzbrojeń | Mniejszy przyrost po kolejnych rearmach |
| Spadek od początku dnia | Bieżący spadek equity względem początku dnia | Mniejszy przyrost podczas straty dziennej |

Każda oś ma siłę, a wartość zero ją wyłącza. Łączny mnożnik jest najmniejszym
z aktywnych mnożników, nie ich iloczynem. Działanie dotyczy nadwyżki ponad
legalny minimalny lot: `minimum + (żądany lot − minimum) × mnożnik`.
Zlecenie już poniżej minimum nie jest automatycznie podnoszone do minimum.
Końcowe ograniczenia wolumenu i brokera nadal obowiązują.

Brak niezbędnego, wiarygodnego stanu nie jest zastępowany zerowym ryzykiem.
Takie nowe zlecenie jest wstrzymywane i otrzymuje przyczynę w diagnostyce.
Nie zmienia to okresu ważności sygnału Synergy. Ważność sygnału i zarządzanie
własnymi zleceniami koszyka pozostają odrębnymi sprawami.

Wolumen uwzględnia cenę konkretnego wejścia. Przy nowym lotowaniu zlecenie
oczekujące musi przejść wspólny warunek strony rynku i minimalnej odległości
od ceny po normalizacji. Kontrola dotyczy symulatora, mostka live i eksperta
MT5. Samo oczekiwanie na odmowę brokera nie wystarcza: dokumentacja MT5
dopuszcza natychmiastowe wykonanie zlecenia oczekującego przy bieżącej cenie,
a warunki zależą od trybu wykonania. Ustawienie `Off` zachowuje dotychczasową
obsługę GOD-X7. Zobacz [typy zleceń w MQL5](https://www.mql5.com/en/book/automation/experts/experts_order_type)
oraz [ograniczenia wolumenu i ceny instrumentu](https://www.mql5.com/en/docs/constants/environment_state/marketinfoconstants).

## Przeplot 300 konfiguracji

Powstaje 150 par. A używa krzywej kapitału i podziału między pozycje; B ma
te same ustawienia oraz od jednej do trzech dodatkowych osi. Wśród B jest
20 wariantów z jedną osią, 65 z dwiema i 65 z trzema. Każdy kandydat ma limit
lot size 5. Lista, ustawienia, dane, program wykonawczy i reguły wyboru są
utrwalane przed przebiegami. Po obejrzeniu wyników lista nie jest uzupełniana
nowymi próbami w miejsce słabszych lub zaokrąglonych do identycznego lota.

TRAIN obejmuje 20 czerwca–31 lipca. Sierpień jest sprawdzany na osobnych
rachunkach od 300 i 600 USD. Wrzesień był już wcześniej analizowany; jego
wynik jest testem przeniesienia, nie nietkniętą próbą. Pełne historyczne okno
z 600 USD służy również porównaniu z GOD-X7. Użyte historyczne wiadomości
zawierają wersje końcowe; nieodnotowane wcześniejsze wersje edycji pozostają
nieznane.

Selekcja uwzględnia DD, dodatnie dni, realną aktywność wejść i zysk. Słaby
wynik nie powoduje automatycznego rozluźnienia warunków. Cel 90–100% dodatnich
dni pozostaje celem pomiaru, nie własnością obiecaną przez mechanizm.

Niezależne przebiegi dni, tygodni, miesięcy i wszystkich możliwych startów
dziennych wymagają rzeczywistego resetu rachunku. Usuwanie najlepszych dni
z gotowej krzywej compoundingu nie zastępuje tych testów. Brak aplikacyjnego
limitu lota również nie usuwa ograniczeń brokera.
