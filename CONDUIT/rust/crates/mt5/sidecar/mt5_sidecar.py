#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
CONDUIT — sidecar MetaTrader 5.

Jeden plik, zero zależności poza pakietem `MetaTrader5` od MetaQuotes.
Łączy się jako KLIENT do gniazda, które otworzył Rust na pętli lokalnej,
i rozmawia protokołem liniowym JSON (jedna linia = jeden dokument, `\\n` kończy).

    żądanie   Rust -> tu :  {"id":7,"cmd":"account","args":{...}}
    odpowiedź tu -> Rust :  {"id":7,"ok":true,"result":{...}}
                            {"id":7,"ok":false,"error":{"code":10016,"msg":"..."}}
    zdarzenie tu -> Rust :  {"ev":"tick","ts":...,"bid":...,"ask":...}
                            {"ev":"closed", ...}

Dlaczego to Rust jest serwerem, a Python klientem: port wybiera system
operacyjny (`bind` na 0), więc nie ma wyścigu o port ani zgadywania, ile
sekund wstaje terminal.

Pętla jest JEDNOWĄTKOWA i to jest decyzja, nie niedopatrzenie. Pakiet
`MetaTrader5` trzyma jedno połączenie IPC do terminala i wołanie go
z kilku wątków potrafi zwrócić dane innego żądania. Zamiast wątków —
`select` z krótkim czasem oczekiwania i odpytywanie ticków między żądaniami.

Uruchamianie ręczne (do diagnostyki, gdy Rust ma autostart=false):
    python mt5_sidecar.py --host 127.0.0.1 --port 51234 --symbol XAUUSD --magic 770077

Hasło do konta NIGDY nie idzie w argumentach (widać je w liście procesów) —
tylko zmienną środowiskową CONDUIT_MT5_PASSWORD.
"""

import argparse
import json
import math
import os
import select
import socket
import sys
import threading
import time
import traceback
from datetime import datetime, timedelta, timezone

PROTO_VERSION = 1
SIDECAR_VERSION = "1.0.2"

# Ile pustych odpytań o tick z rzędu, zanim sprawdzimy, czy terminal żyje.
# Przy domyślnym `--tick-ms 50` to ~2 s ciszy — krócej niż jakakolwiek luka
# w kwotowaniach złota przy otwartym rynku, dłużej niż pojedyncze zacięcie.
BRAK_TICKOW_DO_ODBUDOWY = 40
# Minimalny odstęp między próbami odbudowy (weekend to godziny ciszy —
# nie ma po co restartować w kółko).
ODBUDOWA_CO_S = 30.0
# Co ile sekund sprawdzamy, czy proces bota (rodzic) nadal zyje.
DOZOR_RODZICA_CO_S = 5.0

# --- kody własne (poza dodatnią przestrzenią retcodes MT5) ---
ERR_NOT_INITIALIZED = -1
ERR_UNKNOWN_CMD = -2
ERR_BAD_ARGS = -3
ERR_NO_TICKET = -4
ERR_EXCEPTION = -5
ERR_ACCOUNT_CHANGED = -6
ERR_REAL_NOT_ALLOWED = -7

try:
    import MetaTrader5 as mt5
except ImportError:  # pragma: no cover - zależne od środowiska
    sys.stderr.write(
        "BRAK PAKIETU MetaTrader5. Zainstaluj: pip install MetaTrader5\n"
        "Uwaga: pakiet jest 64-bitowy i TYLKO pod Windows.\n"
    )
    raise


def log(msg):
    sys.stderr.write(str(msg) + "\n")
    sys.stderr.flush()


# ============================================================
#  ZGODNOŚĆ STAŁYCH
# ============================================================
# Nazwy stałych bywają dodawane między wersjami pakietu; czytamy je przez
# getattr z jawną wartością zapasową, żeby stary pakiet nie wywalił startu.

def const(name, fallback):
    return getattr(mt5, name, fallback)


TRADE_ACTION_DEAL = const("TRADE_ACTION_DEAL", 1)
TRADE_ACTION_PENDING = const("TRADE_ACTION_PENDING", 5)
TRADE_ACTION_SLTP = const("TRADE_ACTION_SLTP", 6)
TRADE_ACTION_MODIFY = const("TRADE_ACTION_MODIFY", 7)
TRADE_ACTION_REMOVE = const("TRADE_ACTION_REMOVE", 8)

ORDER_TYPE_BUY = const("ORDER_TYPE_BUY", 0)
ORDER_TYPE_SELL = const("ORDER_TYPE_SELL", 1)

ORDER_TIME_GTC = const("ORDER_TIME_GTC", 0)

ORDER_FILLING_FOK = const("ORDER_FILLING_FOK", 0)
ORDER_FILLING_IOC = const("ORDER_FILLING_IOC", 1)
ORDER_FILLING_RETURN = const("ORDER_FILLING_RETURN", 2)

SYMBOL_FILLING_FOK = const("SYMBOL_FILLING_FOK", 1)
SYMBOL_FILLING_IOC = const("SYMBOL_FILLING_IOC", 2)

POSITION_TYPE_BUY = const("POSITION_TYPE_BUY", 0)

DEAL_ENTRY_IN = const("DEAL_ENTRY_IN", 0)
DEAL_ENTRY_OUT = const("DEAL_ENTRY_OUT", 1)
DEAL_ENTRY_INOUT = const("DEAL_ENTRY_INOUT", 2)
DEAL_ENTRY_OUT_BY = const("DEAL_ENTRY_OUT_BY", 3)

TRADE_RETCODE_DONE = const("TRADE_RETCODE_DONE", 10009)
TRADE_RETCODE_PLACED = const("TRADE_RETCODE_PLACED", 10008)
TRADE_RETCODE_DONE_PARTIAL = const("TRADE_RETCODE_DONE_PARTIAL", 10010)
TRADE_RETCODE_INVALID_FILL = const("TRADE_RETCODE_INVALID_FILL", 10030)
TRADE_RETCODE_REQUOTE = const("TRADE_RETCODE_REQUOTE", 10004)
TRADE_RETCODE_PRICE_CHANGED = const("TRADE_RETCODE_PRICE_CHANGED", 10020)
TRADE_RETCODE_PRICE_OFF = const("TRADE_RETCODE_PRICE_OFF", 10021)

OK_CODES = (TRADE_RETCODE_DONE, TRADE_RETCODE_PLACED, TRADE_RETCODE_DONE_PARTIAL)


# ============================================================
#  INTERWAŁY ŚWIEC
# ============================================================
# Wartość = (stała MT5, długość świecy w sekundach). Długość podajemy SAMI,
# bo z samej stałej się jej nie wyliczy (H1 = 16385, nie 60), a panel musi
# wiedzieć, kiedy przewinąć świecę bieżącą.

TIMEFRAMES = {
    "M1": (const("TIMEFRAME_M1", 1), 60),
    "M2": (const("TIMEFRAME_M2", 2), 120),
    "M3": (const("TIMEFRAME_M3", 3), 180),
    "M4": (const("TIMEFRAME_M4", 4), 240),
    "M5": (const("TIMEFRAME_M5", 5), 300),
    "M6": (const("TIMEFRAME_M6", 6), 360),
    "M10": (const("TIMEFRAME_M10", 10), 600),
    "M12": (const("TIMEFRAME_M12", 12), 720),
    "M15": (const("TIMEFRAME_M15", 15), 900),
    "M20": (const("TIMEFRAME_M20", 20), 1200),
    "M30": (const("TIMEFRAME_M30", 30), 1800),
    "H1": (const("TIMEFRAME_H1", 16385), 3600),
    "H2": (const("TIMEFRAME_H2", 16386), 7200),
    "H3": (const("TIMEFRAME_H3", 16387), 10800),
    "H4": (const("TIMEFRAME_H4", 16388), 14400),
    "H6": (const("TIMEFRAME_H6", 16390), 21600),
    "H8": (const("TIMEFRAME_H8", 16392), 28800),
    "H12": (const("TIMEFRAME_H12", 16396), 43200),
    "D1": (const("TIMEFRAME_D1", 16408), 86400),
    "W1": (const("TIMEFRAME_W1", 32769), 604800),
    # miesiąc nie ma stałej długości — 30 dni to wartość WYŁĄCZNIE do
    # podpowiedzi „kiedy przewinąć świecę", nie do liczenia czasu
    "MN1": (const("TIMEFRAME_MN1", 49153), 2592000),
}

# Panel mówi „5m", MT5 mówi „M5". Tłumaczymy TU, żeby front nie musiał znać
# nazewnictwa MetaQuotes. Klucze wyłącznie małymi literami — porównanie idzie
# po `.lower()`, więc „1m" nie może się pomylić z miesiącem („1mn").
TF_ALIASES = {
    "1m": "M1", "2m": "M2", "3m": "M3", "4m": "M4", "5m": "M5", "6m": "M6",
    "10m": "M10", "12m": "M12", "15m": "M15", "20m": "M20", "30m": "M30",
    "1h": "H1", "2h": "H2", "3h": "H3", "4h": "H4", "6h": "H6", "8h": "H8",
    "12h": "H12", "1d": "D1", "1w": "W1", "1mn": "MN1", "1mo": "MN1",
}

# Sufit na jedno żądanie. Terminal oddaje 40 000 świec M1 w 2 ms, więc nie
# chodzi o jego wydajność — chodzi o to, że pętla sidecara jest JEDNOWĄTKOWA
# i każda milisekunda spędzona na świecach to milisekunda, o którą spóźnia się
# zlecenie stojące w kolejce za nimi. Panel doładowuje historię porcjami.
MAX_BARS = 5000

# To samo dla historii transakcji. Konto testowe ma 1995 dealów z całego życia,
# ale konto po pół roku pracy bota będzie miało dziesiątki tysięcy.
MAX_DEALS = 5000

# Obrona listy symboli. Broker CFD potrafi mieć kilkanaście tysięcy
# instrumentów; powyżej tego progu bez filtra oddajemy tylko widoczne
# w Podglądzie rynku (plus trafienia filtra) — pełna lista w jednej ramce
# protokołu liniowego zatkałaby jednowątkową pętlę sidecara.
MAX_SYMBOLI = 5000

# Kolumny w wierszu deala — kolejność JEST kontraktem, tak samo jak przy
# świecach. Wysyłamy ją w odpowiedzi (`columns`), żeby dało się ją odczytać
# z samej ramki, bez zaglądania w ten plik.
KOLUMNY_DEALA = [
    "ticket", "order", "position", "time_msc", "type", "entry", "volume",
    "price", "profit", "commission", "swap", "fee", "magic", "reason",
    "symbol", "comment",
]

# Ile pomiarów poślizgu trzymamy w pamięci. 2000 zleceń to na tym koncie
# około dwóch dni pracy — wystarcza na sensowny przedział ufności, a nie rośnie
# w nieskończoność w procesie, który ma chodzić tygodniami.
MAX_PROBEK_POSLIZGU = 2000


def timeframe(name):
    """Nazwa interwału -> (stała MT5, długość świecy w sekundach)."""
    raw = str(name or "M5").strip()
    key = TF_ALIASES.get(raw.lower(), raw.upper())
    tf = TIMEFRAMES.get(key)
    if tf is None:
        raise BrokerError(
            ERR_BAD_ARGS,
            "nieznany interwał %r; dopuszczalne: %s" % (name, ", ".join(sorted(TIMEFRAMES)))
        )
    return key, tf[0], tf[1]


class BrokerError(Exception):
    """Odmowa brokera albo błąd sidecara — z kodem, który Rust umie sklasyfikować."""

    def __init__(self, code, msg=""):
        super().__init__(msg)
        self.code = int(code)
        self.msg = str(msg)


def running_terminal_path(configured=None):
    """Attach to an existing process only. Never discover/start a saved registry terminal."""
    import subprocess
    if os.name != "nt":
        raise BrokerError(ERR_NOT_INITIALIZED, "follow-terminal wymaga Windows")
    # Get-Process works without WMI/CIM permissions (CIM was denied in the
    # deployment user's sandbox). Keep PID records so unreadable paths are not
    # silently mistaken for 'only one terminal'. This is read-only discovery.
    command = ("[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); "
               "@(Get-Process -Name terminal64 -ErrorAction SilentlyContinue | "
               "Select-Object Id,Path) | ConvertTo-Json -Compress")
    raw = subprocess.check_output(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", command],
        creationflags=subprocess.CREATE_NO_WINDOW, timeout=10, encoding="utf-8")
    records = json.loads(raw.strip() or "[]")
    if isinstance(records, dict):
        records = [records]
    if not isinstance(records, list) or any(not isinstance(r, dict) or not r.get("Path") for r in records):
        raise BrokerError(ERR_NOT_INITIALIZED, "nie można odczytać ścieżki wszystkich terminali; wybór niepotwierdzony")
    return select_running_terminal([r["Path"] for r in records], configured)


def select_running_terminal(paths, configured=None):
    paths = list(dict.fromkeys(os.path.normcase(os.path.abspath(p)) for p in paths if p))
    if configured:
        wanted = os.path.normcase(os.path.abspath(configured))
        if wanted in paths:
            return wanted
        raise BrokerError(ERR_NOT_INITIALIZED, "wybrany terminal nie jest uruchomiony; follow nie uruchamia MT5")
    if len(paths) != 1:
        raise BrokerError(ERR_NOT_INITIALIZED, "follow wymaga jednego uruchomionego terminala albo jawnej ścieżki")
    return paths[0]


def account_key(account):
    if account is None or not int(getattr(account, "login", 0)) or not str(getattr(account, "server", "")):
        raise BrokerError(ERR_NOT_INITIALIZED, "terminal nie ma potwierdzonego zalogowanego konta")
    return {"login": int(account.login), "server": str(account.server),
            "trade_mode": int(getattr(account, "trade_mode", -1))}


def resolve_gold_symbol(account, candidates):
    """Only exact approved contracts; price is never a broker-identity heuristic."""
    eligible = {name: si for name, si in candidates.items() if si is not None
                and int(getattr(si, "trade_mode", 0)) == 4
                and int(getattr(si, "digits", -1)) >= 0
                and float(getattr(si, "point", 0)) > 0
                and float(getattr(si, "trade_contract_size", 0)) > 0
                and float(getattr(si, "volume_min", 0)) > 0
                and float(getattr(si, "volume_step", 0)) > 0}
    if len(eligible) == 1:
        return next(iter(eligible))
    broker = (str(getattr(account, "company", "")) + " " + str(account.server)).lower()
    vantage = "vantage" in broker
    pu = any(token in broker for token in ("puprime", "pu prime", "pacific union"))
    preferred = "XAUUSD" if vantage and not pu else "XAUUSD.s" if pu and not vantage else None
    if preferred in eligible:
        return preferred
    raise BrokerError(ERR_NOT_INITIALIZED, "niejednoznaczny/brak pełnohandlowego XAUUSD lub XAUUSD.s dla tego brokera")


# ============================================================
#  STAN
# ============================================================

class Sidecar(object):
    def __init__(self, args):
        self.args = args
        self.sock = None
        self.symbol = args.symbol
        self.magic = int(args.magic)
        self.deviation = int(args.deviation)
        self.tick_interval = max(0.005, args.tick_ms / 1000.0)
        self.deal_interval = max(0.05, args.deal_ms / 1000.0)

        self.last_tick_poll = 0.0
        self.last_deal_poll = 0.0
        self.last_send = 0.0
        self.last_tick_key = None          # (time_msc, bid, ask) — nie dublujemy
        # Chart metadata only. A first cached quote does not certify a clock.
        self._quote_clocks = {}
        self._quote_clock_account = None
        self.brak_tickow = 0               # puste odpytania z rzędu (detekcja padu terminala)
        self.ostatnia_odbudowa = 0.0       # kiedy ostatnio wołaliśmy init_terminal()
        self.ostatni_dozor_rodzica = 0.0   # kiedy ostatnio sprawdzalismy proces bota
        try:
            self.parent_pid = os.getppid()
        except Exception:
            self.parent_pid = 0
        self.seen_deals = set()            # tikety dealów już wypchniętych
        self.seen_order = []               # kolejność, żeby ograniczyć zbiór
        self.filling_market = ORDER_FILLING_IOC
        self.filling_pending = ORDER_FILLING_RETURN
        self.point = 0.01
        self.digits = 2
        self.running = True
        self.follow_account = bool(getattr(args, "follow_terminal_account", False))
        self.close_receipt_reconcile = bool(getattr(args, "close_receipt_reconcile", False))
        self.closed_profit_net_costs = bool(getattr(args, "closed_profit_net_costs", False))
        self.cost_receipt_payload = None
        if self.closed_profit_net_costs:
            if not self.close_receipt_reconcile:
                raise ValueError("closed_profit_net_costs requires close_receipt_reconcile")
            # OFF remains a single-file sidecar. ON explicitly requires the
            # versioned helper in its package; missing helper fails before login.
            from cost_contract import cost_receipt_payload
            self.cost_receipt_payload = cost_receipt_payload
        self.allow_real_account = bool(getattr(args, "allow_real_account", False))
        self.bound_account = None
        self.account_changed = False
        self.request_account = None
        # Próbki poślizgu zleceń RYNKOWYCH: (cena wypełnienia − cena żądana),
        # ze znakiem „na niekorzyść klienta". Zbierane na żywo, bo historia
        # terminala tej liczby nie zna — `price_open` zleceń rynkowych jest
        # w niej zerem. Patrz `cmd_costs`.
        self.poslizg = []

    # ---------- terminal ----------

    def init_terminal(self):
        self._reset_quote_clock()
        kwargs = {}
        if self.follow_account:
            kwargs["path"] = running_terminal_path(self.args.terminal)
        elif self.args.terminal:
            kwargs["path"] = self.args.terminal
        if self.args.login and not self.follow_account:
            kwargs["login"] = int(self.args.login)
            pw = os.environ.get("CONDUIT_MT5_PASSWORD")
            if pw:
                kwargs["password"] = pw
            if self.args.server:
                kwargs["server"] = self.args.server
        if not mt5.initialize(**kwargs):
            code, desc = mt5.last_error()
            raise BrokerError(ERR_NOT_INITIALIZED,
                              "initialize() nieudane: %s %s" % (code, desc))
        if self.follow_account:
            account = mt5.account_info()
            key = account_key(account)
            if self.bound_account is not None and key != self.bound_account:
                self.account_changed = True
                raise BrokerError(ERR_ACCOUNT_CHANGED, "konto terminala zmienione; wymagana nowa sesja bota")
            self.bound_account = key
            self.symbol = resolve_gold_symbol(account, {
                name: mt5.symbol_info(name) for name in ("XAUUSD", "XAUUSD.s")})
            self._ensure_account()
        elif self.close_receipt_reconcile:
            # Pin the explicitly logged-in account too, without changing the
            # legacy login/terminal-selection policy. Receipt IDs are account-local.
            key = account_key(mt5.account_info())
            if self.bound_account is not None and key != self.bound_account:
                self.account_changed = True
                raise BrokerError(ERR_ACCOUNT_CHANGED, "konto zmienione; rejestr potwierdzeń wymaga nowej sesji")
            self.bound_account = key
        if not mt5.symbol_select(self.symbol, True):
            raise BrokerError(ERR_NOT_INITIALIZED,
                              "symbol %s niedostępny w Podglądzie rynku" % self.symbol)
        si = mt5.symbol_info(self.symbol)
        if si is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "brak symbol_info dla %s" % self.symbol)
        self.point = si.point
        self.digits = si.digits
        self._pick_filling(si)
        try:
            self._quote_clock_account = account_key(mt5.account_info())
        except BrokerError:
            pass
        log("MT5 gotowy: %s digits=%d point=%s stops=%d filling_mask=%d -> market=%d pending=%d"
            % (self.symbol, si.digits, si.point, si.trade_stops_level,
               si.filling_mode, self.filling_market, self.filling_pending))

    def _ensure_account(self, expected=None, mutation=False):
        if not (self.follow_account or self.close_receipt_reconcile):
            return
        if self.account_changed:
            raise BrokerError(ERR_ACCOUNT_CHANGED, "stara sesja po zmianie konta; polecenie odrzucone")
        current = account_key(mt5.account_info())
        if current != self.bound_account or (expected is not None and current != expected):
            self._reset_quote_clock()
            self.account_changed = True
            raise BrokerError(ERR_ACCOUNT_CHANGED, "login/serwer/typ konta zmieniony; polecenie odrzucone")
        if mutation and expected is None:
            raise BrokerError(ERR_ACCOUNT_CHANGED, "brak tożsamości rachunku w poleceniu handlowym")
        if self.follow_account and mutation and current["trade_mode"] != 0 and not self.allow_real_account:
            raise BrokerError(ERR_REAL_NOT_ALLOWED, "follow: handel poza DEMO wymaga mt5_allow_real_account=true")

    def _guarded_order_send(self, req):
        # Recheck for EVERY retry and ticket mutation, not merely before looking up a ticket.
        self._ensure_account(self.request_account, mutation=True)
        return mt5.order_send(req)

    def _filling_for(self, si):
        """Tryb wypełnienia dla PODANEGO symbolu — bez dotykania stanu sidecara.

        Maska `filling_mode` mówi, co symbol dopuszcza. Dla zleceń rynkowych
        wolimy IOC (częściowe wypełnienie lepsze niż odrzucenie całości przy
        cienkiej książce), dla oczekujących standardem jest RETURN."""
        mask = int(getattr(si, "filling_mode", 0))
        if mask & SYMBOL_FILLING_IOC:
            market = ORDER_FILLING_IOC
        elif mask & SYMBOL_FILLING_FOK:
            market = ORDER_FILLING_FOK
        else:
            market = ORDER_FILLING_RETURN
        return market, ORDER_FILLING_RETURN

    def _pick_filling(self, si):
        """Ustawia tryb wypełnienia sidecara. Wolno wołać WYŁĄCZNIE dla symbolu,
        którym handlujemy — patrz komentarz w `cmd_symbol_info`."""
        self.filling_market, self.filling_pending = self._filling_for(si)

    def _symbol_info(self, sym):
        """`symbol_info` z dociągnięciem symbolu do Podglądu rynku.

        Instrument spoza Podglądu potrafi oddać `symbol_info`, ale nie oddać
        świec — dlatego wybór robimy tu, raz, zamiast zgadywać przy każdym
        odczycie."""
        si = mt5.symbol_info(sym)
        if si is None:
            raise BrokerError(ERR_BAD_ARGS, "broker nie zna symbolu %s" % sym)
        if not getattr(si, "visible", True):
            mt5.symbol_select(sym, True)
            si = mt5.symbol_info(sym) or si
        return si

    # ---------- gniazdo ----------

    def connect(self):
        self.sock = socket.create_connection((self.args.host, self.args.port), timeout=15)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock.settimeout(None)

    def send(self, obj):
        if (self.follow_account or self.close_receipt_reconcile) and obj.get("ev") in ("tick", "closed", "closed_foreign"):
            try:
                self._ensure_account()
            except BrokerError:
                return
            obj["account"] = self.bound_account
        if self.sock is None:
            return
        line = json.dumps(obj, separators=(",", ":"), default=str) + "\n"
        try:
            self.sock.sendall(line.encode("utf-8"))
            self.last_send = time.time()
        except OSError as e:
            log("zapis do Rusta nieudany: %s" % e)
            self.running = False

    def reply_ok(self, rid, result):
        self.send({"id": rid, "ok": True, "result": result})

    def reply_err(self, rid, code, msg):
        self.send({"id": rid, "ok": False, "error": {"code": int(code), "msg": str(msg)[:400]}})

    # ---------- pętla ----------

    def run(self):
        self.connect()
        try:
            self.init_terminal()
        except BrokerError as e:
            # powitanie i tak wysyłamy — Rust ma wiedzieć, że proces żyje,
            # ale nie może handlować
            self.send({"ev": "hello", "proto": PROTO_VERSION,
                       "sidecar": SIDECAR_VERSION, "mt5_version": ""})
            self.send({"ev": "log", "msg": "BŁĄD STARTU: %s" % e.msg})
            log("BŁĄD STARTU: %s" % e.msg)
            time.sleep(2)
            return

        ver = mt5.version()
        self.send({"ev": "hello", "proto": PROTO_VERSION, "sidecar": SIDECAR_VERSION,
                   "mt5_version": ".".join(str(x) for x in ver) if ver else ""})
        self._seed_seen_deals()

        buf = b""
        while self.running:
            if (self.follow_account or self.close_receipt_reconcile) and not self.account_changed:
                try:
                    self._ensure_account()
                except BrokerError as e:
                    # Keep the socket alive long enough to return the explicit identity error.
                    log("FOLLOW ACCOUNT: %s" % e.msg)
                    self.account_changed = True
            try:
                r, _, _ = select.select([self.sock], [], [], 0.01)
            except (OSError, ValueError):
                break
            if r:
                try:
                    chunk = self.sock.recv(65536)
                except OSError:
                    break
                if not chunk:
                    log("Rust zamknął połączenie")
                    break
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    self.handle_line(line)
            now = time.time()
            if now - self.ostatni_dozor_rodzica >= DOZOR_RODZICA_CO_S:
                self.ostatni_dozor_rodzica = now
                if not self._rodzic_zyje():
                    log("proces bota zniknął — sidecar kończy pracę (nie zostaje sierotą)")
                    self.running = False
                    break
            if now - self.last_tick_poll >= self.tick_interval:
                self.last_tick_poll = now
                if not self.account_changed:
                    self.poll_tick()
            if now - self.last_deal_poll >= self.deal_interval:
                self.last_deal_poll = now
                if not self.account_changed:
                    self.poll_deals()
            if now - self.last_send >= 1.0:
                self.send({"ev": "keepalive"})

    def handle_line(self, raw):
        if not raw.strip():
            return
        try:
            req = json.loads(raw.decode("utf-8"))
        except Exception as e:
            log("zły JSON od Rusta: %s" % e)
            return
        rid = req.get("id")
        cmd = req.get("cmd")
        args = req.get("args") or {}
        if rid is None or cmd is None:
            return
        try:
            self.request_account = args.get("_expected_account")
            if cmd != "shutdown":
                self._ensure_account(self.request_account)
            fn = getattr(self, "cmd_" + str(cmd), None)
            if fn is None:
                self.reply_err(rid, ERR_UNKNOWN_CMD, "nieznana komenda: %s" % cmd)
                return
            result = fn(args)
            if cmd != "shutdown":
                self._ensure_account(self.request_account)
            self.reply_ok(rid, result)
        except BrokerError as e:
            self.reply_err(rid, e.code, e.msg)
        except Exception as e:  # pragma: no cover
            log(traceback.format_exc())
            self.reply_err(rid, ERR_EXCEPTION, "%s: %s" % (type(e).__name__, e))
        finally:
            self.request_account = None

    # ---------- strumienie ----------

    def _reset_quote_clock(self):
        self._quote_clocks.clear()
        self._quote_clock_account = None

    def _observe_quote_clock(self, symbol, tick):
        """Pair an advancing quote with its observed UTC; never infer a zone.

        Initial, backwards and same-timestamp snapshots cannot establish or
        refresh evidence. Polling still emits exactly the legacy tick stream.
        The consumer expires the certificate using its monotonic age.
        """
        if self.account_changed or tick is None or not getattr(tick, "time_msc", 0):
            self._quote_clocks.pop(symbol, None)
            return {"quote_observed_utc_ms": None, "quote_observation_age_ms": None}
        now = time.monotonic()
        stamp = int(tick.time_msc)
        previous = self._quote_clocks.get(symbol)
        if previous is None or stamp < previous[0] or now < previous[1]:
            if len(self._quote_clocks) >= 32:
                self._quote_clocks.clear()
            previous = (stamp, now, None, None)
            self._quote_clocks[symbol] = previous
        elif stamp > previous[0]:
            previous = (stamp, now, int(time.time() * 1000), now)
            self._quote_clocks[symbol] = previous
        observed_utc, advanced_at = previous[2:]
        age = max(0, int((now - advanced_at) * 1000)) if advanced_at is not None else None
        return {"quote_observed_utc_ms": observed_utc, "quote_observation_age_ms": age}

    def poll_tick(self):
        t = mt5.symbol_info_tick(self.symbol)
        if t is None:
            self._observe_quote_clock(self.symbol, None)
            # Terminal mógł zniknąć albo zostać zrestartowany pod sidecarem.
            # Uchwyt biblioteki może pozostać pozornie ważny bez nowych ticków,
            # dlatego wykonujemy niezależną kontrolę połączenia.
            self._sprawdz_terminal()
            return
        self.brak_tickow = 0
        self._observe_quote_clock(self.symbol, t)
        key = (t.time_msc, t.bid, t.ask)
        if key == self.last_tick_key:
            return
        self.last_tick_key = key
        self.send({"ev": "tick", "ts": int(t.time_msc), "bid": float(t.bid), "ask": float(t.ask)})

    def _rodzic_zyje(self):
        """Czy proces bota, który nas uruchomił, nadal istnieje.

        Normalnie wystarczy zamknięcie gniazda przez Rusta (`recv` zwraca puste
        dane i pętla się kończy). Ale gdy bota ubije się twardo, a sidecar akurat
        wisi w wywołaniu biblioteki MT5, to zdarzenie przepada i proces zostaje
        SIEROTĄ: trzyma port, uchwyt do terminala i własną sesję MT5. Przy pracy
        z restartami takie procesy mogą się kumulować. Stąd druga, niezależna
        kontrola: samo istnienie procesu-rodzica."""
        ppid = self.parent_pid
        if not ppid:
            return True
        if os.name == "nt":
            # Windows nie ma `kill(pid, 0)`; pytamy jądro o uchwyt do procesu
            # i sprawdzamy jego kod wyjścia (259 = STILL_ACTIVE).
            import ctypes
            PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
            k32 = ctypes.windll.kernel32
            h = k32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, int(ppid))
            if not h:
                return False
            try:
                kod = ctypes.c_ulong()
                if not k32.GetExitCodeProcess(h, ctypes.byref(kod)):
                    return True  # nie wiemy — nie zabijamy się na wszelki wypadek
                return kod.value == 259
            finally:
                k32.CloseHandle(h)
        try:
            os.kill(int(ppid), 0)
            return True
        except OSError:
            return False

    def _sprawdz_terminal(self):
        """Czy terminal po drugiej stronie biblioteki nadal żyje — i odbudowa.

        `mt5.symbol_info_tick` zwraca `None` zarówno przy chwilowym braku
        kwotowania (rynek zamknięty), jak i wtedy, gdy terminal zniknął.
        Rozróżnia je `terminal_info()`: przy martwym procesie zwraca `None`,
        przy żywym-ale-rozłączonym ma `connected == False`. Odbudowę robimy
        dopiero po `BRAK_TICKOW_DO_ODBUDOWY` pustych odpytaniach z rzędu
        i nie częściej niż co `ODBUDOWA_CO_S`, żeby weekend nie wywoływał
        restartu co sekundę."""
        self.brak_tickow += 1
        if self.brak_tickow < BRAK_TICKOW_DO_ODBUDOWY:
            return
        ti = mt5.terminal_info()
        zyje = ti is not None and bool(getattr(ti, "connected", False))
        if zyje:
            # Terminal żyje i jest połączony z brokerem — to zwykła cisza
            # rynku (weekend, przerwa dobowa). Nie restartujemy niczego.
            self.brak_tickow = 0
            return
        now = time.time()
        if now - self.ostatnia_odbudowa < ODBUDOWA_CO_S:
            return
        self.ostatnia_odbudowa = now
        powod = "terminal nie odpowiada" if ti is None else "terminal rozłączony z brokerem"
        log("BRAK TICKÓW (%d prób) — %s; odbudowuję połączenie z terminalem"
            % (self.brak_tickow, powod))
        try:
            mt5.shutdown()
        except Exception:
            pass
        try:
            self.init_terminal()
            self.brak_tickow = 0
            self.last_tick_key = None
            log("połączenie z terminalem odbudowane")
        except BrokerError as e:
            log("odbudowa nieudana: %s" % e.msg)

    def _deal_window(self):
        """Okno czasu do przeglądania historii.

        Historia MT5 jest znakowana czasem SERWERA, a `time.time()` to czas
        lokalny UTC. Różnica bywa kilkugodzinna, więc okno jest celowo szerokie,
        a przed dublami broni zbiór `seen_deals`, nie precyzja zakresu."""
        now = datetime.now(timezone.utc)
        return now - timedelta(hours=26), now + timedelta(hours=26)

    def _seed_seen_deals(self):
        """Przy starcie oznaczamy istniejące deale jako widziane.

        Bez tego po każdym restarcie bot dostałby lawinę „zamknięć" sprzed
        godzin i policzył je jeszcze raz w statystykach."""
        frm, to = self._deal_window()
        deals = mt5.history_deals_get(frm, to)
        if not deals:
            return
        for d in deals:
            self._mark_seen(d.ticket)
        log("historia: %d dealów oznaczonych jako już widziane" % len(deals))

    def _mark_seen(self, ticket):
        if ticket in self.seen_deals:
            return
        self.seen_deals.add(ticket)
        self.seen_order.append(ticket)
        if len(self.seen_order) > 20000:
            old = self.seen_order[:10000]
            self.seen_order = self.seen_order[10000:]
            for t in old:
                self.seen_deals.discard(t)

    def poll_deals(self):
        frm, to = self._deal_window()
        deals = mt5.history_deals_get(frm, to)
        if not deals:
            return
        for d in deals:
            if d.ticket in self.seen_deals:
                continue
            self._mark_seen(d.ticket)
            if d.entry not in (DEAL_ENTRY_OUT, DEAL_ENTRY_OUT_BY) and not (
                self.closed_profit_net_costs and d.entry == DEAL_ENTRY_INOUT
            ):
                continue
            # Cudze transakcje idą OSOBNYM zdarzeniem. Wrzucenie ich do "closed"
            # zafałszowałoby statystyki bota (to jego księga wyników); pominięcie
            # w ogóle — ukryłoby przed użytkownikiem połowę tego, co dzieje się
            # na jego rachunku. Panel łączy oba strumienie, oznaczając źródło.
            ev = "closed" if d.magic == self.magic else "closed_foreign"
            op, ots = self._entry_of_position(d.position_id)
            proof = None
            if self.closed_profit_net_costs:
                # Identity is checked around the read as well as at send. No
                # history from a manually switched account can become a receipt.
                self._ensure_account()
                try:
                    history = mt5.history_deals_get(position=int(d.position_id))
                except Exception:
                    history = None
                info = mt5.account_info()
                self._ensure_account()
                proof = self.cost_receipt_payload(d, history, getattr(info, "currency", None))
            frame = {
                "ev": ev,
                "deal": int(d.ticket),
                "position": int(d.position_id),
                "deal_type": int(d.type),
                "volume": float(d.volume),
                "price": float(d.price),
                "time_msc": int(d.time_msc),
                # ON finite placeholders only keep RawClosed decodable. The
                # separate proof keeps None; it will be quarantined, never net=0.
                "profit": (proof["gross_profit"] or 0.0) if proof is not None else float(d.profit),
                "commission": (proof["exit_commission"] or 0.0) if proof is not None else float(getattr(d, "commission", 0.0)),
                "swap": (proof["swap"] or 0.0) if proof is not None else float(getattr(d, "swap", 0.0)),
                "reason": int(getattr(d, "reason", 0)),
                "magic": int(d.magic),
                "comment": str(d.comment or ""),
                "symbol": str(getattr(d, "symbol", "") or ""),
                "price_open": op,
                "time_open_msc": ots,
            }
            if proof is not None:
                frame["cost_receipt"] = proof
            self.send(frame)

    def _entry_of_position(self, position_id):
        """Cena i czas otwarcia pozycji — z deala wejściowego."""
        try:
            deals = mt5.history_deals_get(position=position_id)
        except Exception:
            return 0.0, 0
        if not deals:
            return 0.0, 0
        for d in deals:
            if d.entry == DEAL_ENTRY_IN:
                return float(d.price), int(d.time_msc)
        return 0.0, 0

    # ============================================================
    #  KOMENDY
    # ============================================================

    def cmd_ping(self, a):
        return {"pong": True, "ts": int(time.time() * 1000)}

    def cmd_shutdown(self, a):
        self.running = False
        return {"bye": True}

    def cmd_subscribe_ticks(self, a):
        sym = a.get("symbol") or self.symbol
        if not mt5.symbol_select(sym, True):
            raise BrokerError(ERR_BAD_ARGS, "nie da się wybrać symbolu %s" % sym)
        if sym != self.symbol:
            self._reset_quote_clock()
        self.symbol = sym
        return {"symbol": sym}

    def cmd_symbol_info(self, a):
        sym = a.get("symbol") or self.symbol
        si = self._symbol_info(sym)
        # Tryb wypełnienia sidecara wolno przestawić TYLKO instrumentem, którym
        # handlujemy. Odkąd panel pyta o parametry dowolnego symbolu (żeby znać
        # `stops_level` przy przeciąganiu SL/TP na wykresie), bezwarunkowe
        # `_pick_filling` ustawiałoby tryb zleceń XAUUSD z maski np. EURUSD —
        # i pierwsze zlecenie po otwarciu wykresu innego instrumentu wracałoby
        # z 10030 INVALID_FILL. Dla obcych symboli liczymy tryb bez zapisu.
        if sym == self.symbol or si.name == self.symbol:
            self._pick_filling(si)
            filling_market, filling_pending = self.filling_market, self.filling_pending
        else:
            filling_market, filling_pending = self._filling_for(si)
        return {
            "symbol": si.name,
            "digits": int(si.digits),
            "point": float(si.point),
            "stops_level_points": float(si.trade_stops_level),
            "freeze_level_points": float(si.trade_freeze_level),
            "volume_min": float(si.volume_min),
            "volume_max": float(si.volume_max),
            "volume_step": float(si.volume_step),
            "contract_size": float(si.trade_contract_size),
            "filling_mask": int(getattr(si, "filling_mode", 0)),
            "filling_market": int(filling_market),
            "filling_pending": int(filling_pending),
            "trade_mode": int(getattr(si, "trade_mode", 0)),
            # --- KOSZT PRZETRZYMANIA (swap) ---
            # Wartości SUROWE z serwera. `swap_mode` mówi, w czym są wyrażone:
            # 1 = SYMBOL_SWAP_MODE_POINTS (punkty), i tak jest na Vantage.
            # Przeliczenie na dolary wymaga `tick_value`/`tick_size` — dlatego
            # jadą razem, a nie osobno. Zmierzone 29.07 na XAUUSD: swap_long
            # −75,82 pkt, tick_value 1,00 $, tick_size 0,01 = point, czyli
            # 1 punkt = 1,00 $ na lota i swap_long = −75,82 $ za lota za dobę.
            # NIE WOLNO tego zaszywać w symulatorze: EURUSD ma −5,97,
            # XAGUSD −23,65, a broker zmienia te stawki bez uprzedzenia.
            "swap_long": float(getattr(si, "swap_long", 0.0)),
            "swap_short": float(getattr(si, "swap_short", 0.0)),
            "swap_mode": int(getattr(si, "swap_mode", 0)),
            # dzień tygodnia liczony potrójnie (3 = środa na Vantage)
            "swap_rollover3days": int(getattr(si, "swap_rollover3days", 0)),
            "tick_value": float(getattr(si, "trade_tick_value", 0.0)),
            "tick_size": float(getattr(si, "trade_tick_size", 0.0)),
            # `visible` = czy instrument jest w Podglądzie rynku. Niewidoczny
            # potrafi oddać parametry i nie oddać świec.
            "visible": bool(getattr(si, "visible", True)),
            "description": str(getattr(si, "description", "") or ""),
        }

    # ---------- historia rachunku ----------

    def cmd_history_deals(self, a):
        """PEŁNA historia transakcji z dowolnego zakresu dat.

        Po co osobne polecenie, skoro sidecar i tak strumieniuje `closed`:
        strumień celowo pokazuje TYLKO to, co wydarzyło się od uruchomienia
        (patrz `_seed_seen_deals` — bez tego po każdym restarcie bot policzyłby
        wczorajsze zamknięcia jeszcze raz w statystykach). To jest właściwe
        zachowanie dla STATYSTYK BOTA i błędne dla EKSPORTU HISTORII: eksport
        oddawał garść rekordów z bieżącej sesji przy koncie, które ma ich 989,
        i nie mówił o własnej niekompletności ani słowem.

        To polecenie jest drugą, niezależną drogą: czyta wprost z terminala,
        nie dotyka `seen_deals` i nie wpływa na żaden licznik silnika.

        Zakres podaje się w milisekundach epoki, w CZASIE SERWERA BROKERA —
        tej samej bazie, w której jadą świece, ticki i `time_msc` pozycji.
        Brak `from` = od początku istnienia rachunku.
        """
        try:
            do_ms = int(a["to"]) if a.get("to") is not None else None
            od_ms = int(a["from"]) if a.get("from") is not None else None
        except (TypeError, ValueError):
            raise BrokerError(ERR_BAD_ARGS, "from/to muszą być epoką w milisekundach")

        # Rachunek MT5 nie istniał przed 1970 r., a górną granicę trzeba dać
        # z zapasem na przesunięcie zegara serwera — stąd doba do przodu.
        od = datetime.fromtimestamp(od_ms / 1000.0, tz=timezone.utc) if od_ms is not None \
            else datetime(1970, 1, 2, tzinfo=timezone.utc)
        do = datetime.fromtimestamp(do_ms / 1000.0, tz=timezone.utc) if do_ms is not None \
            else datetime.now(timezone.utc) + timedelta(days=1)

        deals = mt5.history_deals_get(od, do)
        if deals is None:
            code, desc = mt5.last_error()
            # Pusty zakres to nie awaria — brak historii w oknie jest
            # prawidłową odpowiedzią. Awaria to dopiero błąd terminala.
            if code in (1, 0):
                deals = ()
            else:
                raise BrokerError(ERR_NOT_INITIALIZED,
                                  "terminal nie oddał historii: %s %s" % (code, desc))

        sym = a.get("symbol")
        magic = a.get("magic")
        tylko_wyjscia = bool(a.get("out_only"))

        # Literówka w nazwie instrumentu nie może wyglądać jak „konto nie ma
        # tam historii". Pusty eksport z powodu `XAUUSD ` ze spacją byłby
        # nie do odróżnienia od prawdziwie pustego zakresu — a to jest ta sama
        # klasa cichej porażki, którą naprawia całe to polecenie.
        # Zakres bez transakcji dalej zwraca pustą listę, bo to jest PRAWDA.
        if sym:
            self._symbol_info(sym)

        wybrane = []
        for d in deals:
            if sym and str(getattr(d, "symbol", "")) != sym:
                continue
            if magic is not None and int(d.magic) != int(magic):
                continue
            if tylko_wyjscia and d.entry not in (DEAL_ENTRY_OUT, DEAL_ENTRY_OUT_BY):
                continue
            wybrane.append(d)

        wybrane.sort(key=lambda d: (d.time_msc, d.ticket))
        total = len(wybrane)

        offset = max(0, int(a.get("offset") or 0))
        limit = int(a.get("limit") or MAX_DEALS)
        limit = max(1, min(MAX_DEALS, limit))
        okno = wybrane[offset:offset + limit]

        wiersze = [
            [int(d.ticket), int(d.order), int(d.position_id), int(d.time_msc),
             int(d.type), int(d.entry), float(d.volume), float(d.price),
             float(d.profit), float(getattr(d, "commission", 0.0)),
             float(getattr(d, "swap", 0.0)), float(getattr(d, "fee", 0.0)),
             int(d.magic), int(getattr(d, "reason", 0)),
             str(getattr(d, "symbol", "") or ""), str(d.comment or "")]
            for d in okno
        ]
        return {
            "columns": KOLUMNY_DEALA,
            "total": total,
            "offset": offset,
            "count": len(wiersze),
            # `true`, gdy za tym oknem są jeszcze rekordy — odbiorca wie, że ma
            # dopytać, zamiast uznać niepełną listę za całą historię
            "more": offset + len(wiersze) < total,
            "deals": wiersze,
        }

    def cmd_costs(self, a):
        """Zmierzone koszty wykonania: poślizg rynkowy i wypełnienia oczekujących.

        Dwa źródła, bo dwie różne rzeczy i różna dostępność danych:

        * **oczekujące** — z historii terminala. `history_orders_get` trzyma
          poziom aktywacji w `price_open`, a deal cenę wypełnienia; różnica
          jest poślizgiem. Zmierzone 29.07 na 646 wypełnieniach: DOKŁADNIE
          zero w każdym pojedynczym przypadku.
        * **rynkowe** — z historii NIE DA SIĘ. MT5 zapisuje `price_open = 0.00`
          dla zleceń rynkowych (sprawdzone na 344 wejściach), a `price_current`
          to już cena wykonania. Jedynym miejscem, które zna cenę ŻĄDANĄ, jest
          ten proces w chwili `order_send` — dlatego zbieramy próbki na żywo
          w `self.poslizg`.

        Zwracamy PRÓBKI I LICZNOŚĆ, nie samą średnią: liczba bez próbki nie
        jest wynikiem, a symulator ma prawo wiedzieć, na czym stoi.
        """
        okno_dni = float(a.get("days") or 40.0)
        do = datetime.now(timezone.utc) + timedelta(days=1)
        od = do - timedelta(days=okno_dni + 1)

        oczek = {"n": 0, "zero": 0, "suma": 0.0, "max_abs": 0.0}
        try:
            deals = mt5.history_deals_get(od, do) or ()
            orders = mt5.history_orders_get(od, do) or ()
            po_tickecie = {o.ticket: o for o in orders}
            for d in deals:
                if d.entry != DEAL_ENTRY_IN:
                    continue
                o = po_tickecie.get(d.order)
                # typy 2..5 to zlecenia oczekujące; rynkowe (0/1) mają
                # `price_open == 0` i nie ma ich z czym porównać
                if o is None or o.type not in (2, 3, 4, 5) or o.price_open <= 0:
                    continue
                # dodatni = wypełnienie NA NIEKORZYŚĆ klienta
                znak = 1.0 if d.type == ORDER_TYPE_BUY else -1.0
                r = (float(d.price) - float(o.price_open)) * znak
                oczek["n"] += 1
                oczek["suma"] += r
                if abs(r) < 1e-9:
                    oczek["zero"] += 1
                oczek["max_abs"] = max(oczek["max_abs"], abs(r))
        except Exception as e:  # pragma: no cover
            log("pomiar poślizgu oczekujących nieudany: %s" % e)

        return {
            "symbol": self.symbol,
            "window_days": okno_dni,
            "pending": {
                "n": oczek["n"],
                "mean": (oczek["suma"] / oczek["n"]) if oczek["n"] else 0.0,
                "exact_at_level": oczek["zero"],
                "max_abs": oczek["max_abs"],
                "source": "history",
            },
            "market": self._poslizg_rynkowy(),
        }

    def _zapisz_poslizg(self, zadana, wypelniona, side):
        """Zapamiętuje jedną próbkę poślizgu zlecenia rynkowego.

        Dodatni = gorzej dla nas: BUY wypełniony wyżej, SELL niżej."""
        if not (zadana > 0 and wypelniona > 0):
            return
        znak = 1.0 if side == "buy" else -1.0
        self.poslizg.append((wypelniona - zadana) * znak)
        if len(self.poslizg) > MAX_PROBEK_POSLIZGU:
            del self.poslizg[:len(self.poslizg) - MAX_PROBEK_POSLIZGU]

    def _poslizg_rynkowy(self):
        v = list(self.poslizg)
        n = len(v)
        if n == 0:
            return {"n": 0, "mean": 0.0, "sd": 0.0, "ci95": 0.0,
                    "exact": 0, "max_abs": 0.0, "source": "live"}
        sr = sum(v) / n
        war = sum((x - sr) ** 2 for x in v) / (n - 1) if n > 1 else 0.0
        sd = war ** 0.5
        return {
            "n": n,
            "mean": sr,
            "sd": sd,
            "ci95": 1.96 * sd / (n ** 0.5) if n > 1 else 0.0,
            "exact": sum(1 for x in v if abs(x) < 1e-9),
            "max_abs": max(abs(x) for x in v),
            "source": "live",
        }

    # ---------- świece ----------

    def cmd_candles(self, a):
        """Świece OHLC prosto z terminala.

        Dwie drogi, obie z `copy_rates_*`:
          * bez `to` — `copy_rates_from_pos(sym, tf, 0, count)`, czyli `count`
            najnowszych świec ze świecą BIEŻĄCĄ (jeszcze niedomkniętą) na końcu;
          * z `to` — `copy_rates_from(sym, tf, to-1s, count)`, czyli `count`
            świec STARSZYCH niż `to`. Kotwicą `copy_rates_from` jest świeca
            OSTATNIA, a seria biegnie w przeszłość.
            To jest droga doładowywania historii przy przewijaniu w lewo.

        ZEGAR. Surowe `time` świecy i `time_msc` ticku zwracamy bez zmiany
        istniejącej osi wykresu. Ostatni tick może pochodzić z zamkniętej
        sesji. Offset wolno wyznaczać tylko z pary zarejestrowanej podczas
        postępu kwotowań, nigdy odejmując dzisiejsze UTC od starego ticku.
        Ta obserwacja nie zmienia strefy ani ustawień silnika.
        """
        sym = a.get("symbol") or self.symbol
        tf_name, tf, bar_s = timeframe(a.get("tf"))
        try:
            count = int(a.get("count") or 500)
        except (TypeError, ValueError):
            raise BrokerError(ERR_BAD_ARGS, "count musi być liczbą")
        count = max(1, min(MAX_BARS, count))

        si = self._symbol_info(sym)

        to = a.get("to")
        if to is None:
            rates = mt5.copy_rates_from_pos(sym, tf, 0, count)
        else:
            try:
                to_ms = int(to)
            except (TypeError, ValueError):
                raise BrokerError(ERR_BAD_ARGS, "to musi być epoką w milisekundach")
            # granica WYŁĄCZNA: panel podaje czas najstarszej świecy, którą już
            # ma, i chce wyłącznie starsze. Sekunda wstecz wypada w poprzedniej
            # świecy przy każdym interwale (najkrótszy to 60 s).
            kotwica = datetime.fromtimestamp(to_ms / 1000.0 - 1.0, tz=timezone.utc)
            rates = mt5.copy_rates_from(sym, tf, kotwica, count)

        if rates is None:
            code, desc = mt5.last_error()
            raise BrokerError(
                ERR_NOT_INITIALIZED,
                "terminal nie oddał świec %s %s: %s %s" % (sym, tf_name, code, desc)
            )

        bars = [
            [int(r["time"]) * 1000, float(r["open"]), float(r["high"]),
             float(r["low"]), float(r["close"]), int(r["tick_volume"]),
             int(r["spread"])]
            for r in rates
        ]

        # Read-only account check prevents a cached clock crossing an account
        # switch even in legacy non-FOLLOW mode. No credentials leave memory.
        try:
            account = account_key(mt5.account_info())
        except BrokerError:
            account = None
        if account != self._quote_clock_account or account is None:
            self._reset_quote_clock()
            self._quote_clock_account = account
        t = mt5.symbol_info_tick(sym)
        server_ms = int(t.time_msc) if t is not None and t.time_msc else None
        observation = self._observe_quote_clock(si.name, t)

        return {
            "symbol": si.name,
            "tf": tf_name,
            "bar_ms": bar_s * 1000,
            "digits": int(si.digits),
            "point": float(si.point),
            "server_time_ms": server_ms,
            "utc_time_ms": int(time.time() * 1000),
            **observation,
            # kolumny: t, o, h, l, c, tick_volume, spread(pkt)
            "bars": bars,
        }

    def cmd_account(self, a):
        ai = mt5.account_info()
        if ai is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "brak account_info")
        # TOŻSAMOŚĆ konta jedzie razem z liczbami i to jest celowe: panel ma
        # pokazywać, NA KTÓRYM koncie bot handluje. Bez tego jedyną widoczną
        # różnicą między demo a rachunkiem realnym jest wysokość salda —
        # a to jest dokładnie ta pomyłka, której nie wolno dać popełnić.
        # `trade_mode`: 0 = DEMO, 1 = KONKURS, 2 = REALNE.
        #
        # KREDYT BONUSOWY (`ACCOUNT_CREDIT`) jest JUŻ WLICZONY w `balance`:
        # wpłata 300 $ z bonusem 100 % daje balance 600, credit 300. Bez tej
        # liczby Rust widzi konto na 600 $ i liczy lot od cudzych pieniędzy.
        #
        # `getattr` z zerem, a nie `ai.credit`: pole jest w MetaTrader5 od
        # dawna, ale sidecar musi wstać także na builda, w którym go nie ma —
        # brak bonusu nie jest powodem, żeby most nie ruszył. Zero = konto bez
        # kredytu = podstawa lota równa saldu.
        return {
            "balance": float(ai.balance),
            "equity": float(ai.equity),
            "margin": float(ai.margin),
            "margin_free": float(ai.margin_free),
            "credit": float(getattr(ai, "credit", 0.0) or 0.0),
            "leverage": int(ai.leverage),
            "currency": str(ai.currency),
            "login": int(ai.login),
            "server": str(ai.server),
            "company": str(ai.company),
            "holder": str(ai.name),
            "trade_mode": int(getattr(ai, "trade_mode", 0)),
        }


    def cmd_symbols(self, a):
        """Lista symboli brokera do wyszukiwarki instrumentów w panelu.

        Nazwy i sufiksy symboli różnią się między brokerami, dlatego źródłem
        prawdy jest bieżący terminal, a nie statyczny katalog.

        `q` (przyjmowany tez jako `filter`) zaweza po podciagu, bez
        rozrozniania wielkosci liter. Obrona: gdy broker ma > MAX_SYMBOLI
        instrumentow, oddajemy WIDOCZNE w Podgladzie rynku plus trafienia
        filtra — a nie pelna liste tysiecy CFD w jednej ramce protokolu.
        Sortowanie ZAWSZE przed jakimkolwiek cieciem: widoczne najpierw,
        potem alfabetycznie (wczesniej sort byl PO obcieciu do 200, wiec
        obcinal poczatek alfabetu razem z widocznymi).

        `digits` i `trade_mode` (SYMBOL_TRADE_MODE_*, int) jada od razu:
        panel formatuje ceny i gasi instrumenty z wylaczonym handlem bez
        drugiej rundy zapytan o kazdy symbol z osobna.
        """
        filtr = str(a.get("q") or a.get("filter") or "").strip().lower()
        wszystkie = mt5.symbols_get()
        if wszystkie is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "terminal nie oddal listy symboli")
        duzo = len(wszystkie) > MAX_SYMBOLI
        out = []
        for s in wszystkie:
            widoczny = bool(getattr(s, "visible", False))
            pasuje = bool(filtr) and filtr in s.name.lower()
            if duzo:
                if not (widoczny or pasuje):
                    continue
            elif filtr and not pasuje:
                continue
            out.append({
                "name": s.name,
                "visible": widoczny,
                "digits": int(getattr(s, "digits", 0)),
                "trade_mode": int(getattr(s, "trade_mode", 0)),
            })
        # widoczne w Podgladzie rynku najpierw, potem alfabetycznie
        out.sort(key=lambda x: (not x["visible"], x["name"]))
        # `total` = ile broker ma NAPRAWDE — odbiorca widzi, czy lista
        # jest pelna, czy przycieta obrona MAX_SYMBOLI
        return {"symbols": out, "total": len(wszystkie)}

    def cmd_quote(self, a):
        sym = a.get("symbol") or self.symbol
        t = mt5.symbol_info_tick(sym)
        if t is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "brak kwotowania %s" % sym)
        return {"ts": int(t.time_msc), "bid": float(t.bid), "ask": float(t.ask)}

    def cmd_positions(self, a):
        # `all` = zwróć rachunek W CAŁOŚCI, nie tylko symbol bota. CONDUIT jest
        # podglądem całego konta; pozycje na innych instrumentach też zużywają
        # margines i wchodzą w equity, więc ich pominięcie robiło z panelu
        # kłamcę. Co z tego bot ZARZĄDZA, decyduje dopiero `is_ours` w Ruście.
        sym = a.get("symbol") or self.symbol
        ps = mt5.positions_get() if a.get("all") else mt5.positions_get(symbol=sym)
        if self.close_receipt_reconcile and ps is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "odczyt positions_get nieudany; None nie jest pustym snapshotem")
        out = []
        for p in ps or ():
            out.append({
                "ticket": int(p.ticket),
                "identifier": int(getattr(p, "identifier", 0) or 0),
                "kind": 0 if p.type == POSITION_TYPE_BUY else 1,
                "volume": float(p.volume),
                "price_open": float(p.price_open),
                "time_msc": int(p.time_msc),
                "sl": float(p.sl),
                "tp": float(p.tp),
                "profit": float(p.profit),
                "magic": int(p.magic),
                "comment": str(p.comment or ""),
                "symbol": str(p.symbol),
            })
        return out

    def cmd_orders(self, a):
        # `all` — jak w `cmd_positions`
        sym = a.get("symbol") or self.symbol
        os_ = mt5.orders_get() if a.get("all") else mt5.orders_get(symbol=sym)
        if self.close_receipt_reconcile and os_ is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "odczyt orders_get nieudany; None nie jest pustym snapshotem")
        out = []
        for o in os_ or ():
            out.append({
                "ticket": int(o.ticket),
                "kind": int(o.type),
                "volume": float(o.volume_current),
                "price_open": float(o.price_open),
                "time_msc": int(o.time_setup_msc),
                "sl": float(o.sl),
                "tp": float(o.tp),
                "magic": int(o.magic),
                "comment": str(o.comment or ""),
                "symbol": str(o.symbol),
            })
        return out

    # ---------- handel ----------

    def _send_order(self, req, fillings):
        """Wysyła zlecenie, przechodząc po dopuszczalnych trybach wypełnienia.

        10030 (INVALID_FILL) to jedyny błąd, który da się naprawić bez pytania
        człowieka: broker po prostu nie akceptuje trybu, który wybraliśmy
        z maski. Reszta odmów leci wyżej bez zmian."""
        last = None
        for f in fillings:
            req["type_filling"] = f
            res = self._guarded_order_send(req)
            if res is None:
                code, desc = mt5.last_error()
                raise BrokerError(ERR_EXCEPTION, "order_send zwrócił None: %s %s" % (code, desc))
            last = res
            if res.retcode in OK_CODES:
                # zapamiętaj działający tryb — kolejne zlecenia pójdą od razu
                if req.get("action") == TRADE_ACTION_PENDING:
                    self.filling_pending = f
                else:
                    self.filling_market = f
                return res
            if res.retcode != TRADE_RETCODE_INVALID_FILL:
                break
        raise BrokerError(last.retcode, "%s" % (last.comment or ""))

    def _fillings_for(self, action):
        if action == TRADE_ACTION_PENDING:
            first = self.filling_pending
            rest = [ORDER_FILLING_RETURN, ORDER_FILLING_IOC, ORDER_FILLING_FOK]
        else:
            first = self.filling_market
            rest = [ORDER_FILLING_IOC, ORDER_FILLING_FOK, ORDER_FILLING_RETURN]
        out = [first] + [x for x in rest if x != first]
        return out

    def _position_of_deal(self, deal_ticket):
        """Tiket POZYCJI dla świeżo zawartego deala.

        `order` z `order_send` to tiket ZLECENIA, nie pozycji. Przy zleceniu
        rynkowym otwierającym nową pozycję bywają równe, ale przy dołożeniu do
        istniejącej pozycji (netting) już nie — i wtedy bot zarządzałby
        nieistniejącym tiketem."""
        for _ in range(20):
            try:
                ds = mt5.history_deals_get(ticket=deal_ticket)
            except Exception:
                ds = None
            if ds:
                return int(ds[0].position_id)
            time.sleep(0.01)
        return 0

    def _exact_ack_deal(self, order_ticket, deal_ticket, symbol, deal_type,
                        entries, position_identifier=None, *, ack_volume, ack_price):
        """Strict identity/geometry proof: ticket history filters ORDER, not DEAL.

        Only read queries are retried. A missing/different/ambiguous record is
        unknown, never the first deal of another order and never ticket=id.
        This is session evidence, not durable receipt-consumer acknowledgement.
        """
        valid_id = lambda value: isinstance(value, int) and not isinstance(value, bool) and value > 0
        # Some MT5 brokers return MqlTradeResult.order=0 for an executed market
        # close.  A close still has stronger, non-guessing evidence available:
        # the stable position identifier captured before order_send plus the
        # unique deal ticket returned by order_send.  Query that position's
        # history and select the exact deal instead of rejecting a valid close
        # merely because the broker omitted the transient order ticket.
        if not valid_id(deal_ticket):
            return None
        if position_identifier is not None and not valid_id(position_identifier):
            return None
        if position_identifier is None and not valid_id(order_ticket):
            return None
        def valid_number(value):
            return (isinstance(value, (int, float)) and not isinstance(value, bool)
                    and math.isfinite(value) and value > 0)
        if not valid_number(ack_volume) or not valid_number(ack_price):
            return None
        def same_number(actual, confirmed):
            # Only binary round-off, not tick/lot rounding or an order-total
            # heuristic. MqlTradeResult volume/price describe its DEAL.
            return (valid_number(actual) and abs(actual - confirmed)
                    <= 4 * max(math.ulp(actual), math.ulp(confirmed)))
        for attempt in range(20):
            self._ensure_account(self.request_account)
            try:
                if position_identifier is not None:
                    rows = mt5.history_deals_get(position=position_identifier)
                else:
                    rows = mt5.history_deals_get(ticket=order_ticket)
            except Exception:
                rows = None
            self._ensure_account(self.request_account)
            if rows is not None:
                try:
                    # ORDER-filter queries must contain only that order.  A
                    # POSITION-filter query intentionally contains the whole
                    # lifecycle; select the unique exact deal below and then
                    # validate its order whenever the broker supplied one.
                    if (position_identifier is None
                            and any(not valid_id(d.order) or d.order != order_ticket
                                    or not valid_id(d.ticket) for d in rows)):
                        return None
                    exact = [d for d in rows if d.ticket == deal_ticket]
                    if len(exact) > 1:
                        return None
                    if len(exact) == 1:
                        d = exact[0]
                        identifier = d.position_id
                        if (not valid_id(identifier) or d.symbol != symbol
                                or not isinstance(d.type, int) or isinstance(d.type, bool)
                                or d.type != deal_type
                                or not isinstance(d.entry, int) or isinstance(d.entry, bool)
                                or d.entry not in entries or d.magic != self.magic
                                or (valid_id(order_ticket) and d.order != order_ticket)
                                or not same_number(d.volume, ack_volume)
                                or not same_number(d.price, ack_price)
                                or (position_identifier is not None
                                    and identifier != position_identifier)):
                            return None
                        return d
                except (AttributeError, TypeError, ValueError, OverflowError):
                    return None
            if attempt < 19:
                time.sleep(0.01)
        return None

    def _physical_position_from_entry(self, d, symbol, position_type):
        """Map a proven DEAL_POSITION_ID to one current, owned physical ticket."""
        if d is None:
            return 0
        self._ensure_account(self.request_account)
        try:
            rows = mt5.positions_get(symbol=symbol)
        except Exception:
            rows = None
        self._ensure_account(self.request_account)
        if rows is None:
            return 0
        try:
            matches = [p for p in rows if p.identifier == d.position_id]
            if len(matches) != 1:
                return 0
            p = matches[0]
            if (not isinstance(p.ticket, int) or isinstance(p.ticket, bool) or p.ticket <= 0
                    or not isinstance(p.identifier, int) or isinstance(p.identifier, bool)
                    or p.symbol != symbol or not isinstance(p.type, int) or isinstance(p.type, bool)
                    or p.type != position_type or p.magic != self.magic
                    or not math.isfinite(float(p.volume)) or float(p.volume) <= 0):
                return 0
            return int(p.ticket)
        except (AttributeError, TypeError, ValueError, OverflowError):
            return 0

    def _base(self, action, **kw):
        r = {"action": action, "symbol": self.symbol, "magic": self.magic,
             "type_time": ORDER_TIME_GTC}
        r.update(kw)
        return r

    def cmd_open_market(self, a):
        side = a.get("side")
        vol = float(a.get("volume", 0))
        if side not in ("buy", "sell") or vol <= 0:
            raise BrokerError(ERR_BAD_ARGS, "zła strona albo wolumen")
        t = mt5.symbol_info_tick(self.symbol)
        if t is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "brak kwotowania")
        zadana = float(t.ask if side == "buy" else t.bid)
        req = self._base(
            TRADE_ACTION_DEAL,
            volume=vol,
            type=ORDER_TYPE_BUY if side == "buy" else ORDER_TYPE_SELL,
            price=zadana,
            deviation=self.deviation,
            comment=str(a.get("comment") or "")[:31],
        )
        if a.get("sl") is not None:
            req["sl"] = float(a["sl"])
        if a.get("tp") is not None:
            req["tp"] = float(a["tp"])
        res = self._send_order(req, self._fillings_for(TRADE_ACTION_DEAL))
        # JEDYNE miejsce w całym systemie, które zna obie ceny naraz: żądaną
        # (z kwotowania sprzed wysyłki) i uzyskaną. Historia terminala tego
        # nie zapamięta, więc jak tego tu nie zapiszemy, to nie zapisze tego
        # nikt — a symulator zostanie ze zgadywaną stałą.
        self._zapisz_poslizg(zadana, float(res.price), side)
        if self.close_receipt_reconcile:
            d = self._exact_ack_deal(res.order, res.deal, self.symbol,
                ORDER_TYPE_BUY if side == "buy" else ORDER_TYPE_SELL, (DEAL_ENTRY_IN,),
                ack_volume=res.volume, ack_price=res.price)
            identifier = int(d.position_id) if d is not None else 0
            pos = self._physical_position_from_entry(d, self.symbol,
                ORDER_TYPE_BUY if side == "buy" else ORDER_TYPE_SELL)
        else:
            # Exact legacy branch: OFF keeps its old query and return fields.
            pos = self._position_of_deal(int(res.deal)) if res.deal else int(res.order)
            identifier = pos if res.deal else 0
        return {
            "retcode": int(res.retcode),
            "order": int(res.order),
            "deal": int(res.deal),
            "position": int(pos),
            "position_identifier": int(identifier),
            "volume": float(res.volume),
            "price": float(res.price),
            "comment": str(res.comment or ""),
        }

    def cmd_place_pending(self, a):
        kind = int(a.get("kind", -1))
        if kind not in (2, 3, 4, 5):
            raise BrokerError(ERR_BAD_ARGS, "zły typ zlecenia oczekującego: %s" % kind)
        vol = float(a.get("volume", 0))
        price = float(a.get("price", 0))
        if vol <= 0 or price <= 0:
            raise BrokerError(ERR_BAD_ARGS, "zły wolumen albo cena")
        req = self._base(
            TRADE_ACTION_PENDING,
            volume=vol,
            type=kind,
            price=price,
            comment=str(a.get("comment") or "")[:31],
        )
        if a.get("sl") is not None:
            req["sl"] = float(a["sl"])
        if a.get("tp") is not None:
            req["tp"] = float(a["tp"])
        res = self._send_order(req, self._fillings_for(TRADE_ACTION_PENDING))
        return {
            "retcode": int(res.retcode),
            "order": int(res.order),
            "deal": int(res.deal),
            "position": 0,
            "volume": float(res.volume),
            "price": float(res.price),
            "comment": str(res.comment or ""),
        }

    def _position(self, ticket):
        ps = mt5.positions_get(ticket=int(ticket))
        if not ps:
            raise BrokerError(ERR_NO_TICKET, "brak pozycji %s" % ticket)
        return ps[0]

    def cmd_modify_position(self, a):
        p = self._position(a.get("ticket"))
        req = {
            "action": TRADE_ACTION_SLTP,
            "symbol": p.symbol,
            "position": int(p.ticket),
            "sl": float(a["sl"]) if a.get("sl") is not None else 0.0,
            "tp": float(a["tp"]) if a.get("tp") is not None else 0.0,
        }
        res = self._guarded_order_send(req)
        if res is None:
            code, desc = mt5.last_error()
            raise BrokerError(ERR_EXCEPTION, "order_send None: %s %s" % (code, desc))
        if res.retcode not in OK_CODES:
            raise BrokerError(res.retcode, str(res.comment or ""))
        return {"retcode": int(res.retcode), "order": int(res.order), "deal": 0,
                "position": int(p.ticket), "volume": 0.0, "price": 0.0, "profit": 0.0}

    def cmd_modify_pending(self, a):
        ticket = int(a.get("ticket", 0))
        os_ = mt5.orders_get(ticket=ticket)
        if not os_:
            raise BrokerError(ERR_NO_TICKET, "brak zlecenia %s" % ticket)
        req = {
            "action": TRADE_ACTION_MODIFY,
            "order": ticket,
            "price": float(a.get("price", 0)),
            "sl": float(a["sl"]) if a.get("sl") is not None else 0.0,
            "tp": float(a["tp"]) if a.get("tp") is not None else 0.0,
            "type_time": ORDER_TIME_GTC,
        }
        res = self._guarded_order_send(req)
        if res is None:
            code, desc = mt5.last_error()
            raise BrokerError(ERR_EXCEPTION, "order_send None: %s %s" % (code, desc))
        if res.retcode not in OK_CODES:
            raise BrokerError(res.retcode, str(res.comment or ""))
        return {"retcode": int(res.retcode), "order": ticket, "deal": 0, "position": 0,
                "volume": 0.0, "price": float(a.get("price", 0)), "profit": 0.0}

    def _close(self, ticket, volume):
        p = self._position(ticket)
        t = mt5.symbol_info_tick(p.symbol)
        if t is None:
            raise BrokerError(ERR_NOT_INITIALIZED, "brak kwotowania")
        is_buy = p.type == POSITION_TYPE_BUY
        req = {
            "action": TRADE_ACTION_DEAL,
            "symbol": p.symbol,
            "position": int(p.ticket),
            "volume": float(volume),
            # zamykamy zleceniem PRZECIWNYM: pozycję BUY zamyka SELL po BID
            "type": ORDER_TYPE_SELL if is_buy else ORDER_TYPE_BUY,
            "price": float(t.bid if is_buy else t.ask),
            "deviation": self.deviation,
            "magic": self.magic,
            "type_time": ORDER_TIME_GTC,
            "comment": "close",
        }
        res = self._send_order(req, self._fillings_for(TRADE_ACTION_DEAL))
        if self.close_receipt_reconcile:
            identifier = getattr(p, "identifier", 0)
            d = self._exact_ack_deal(res.order, res.deal, p.symbol,
                ORDER_TYPE_SELL if is_buy else ORDER_TYPE_BUY,
                (DEAL_ENTRY_OUT, DEAL_ENTRY_OUT_BY), identifier,
                ack_volume=res.volume, ack_price=res.price)
            components = None
            if d is not None:
                try:
                    values = {"gross_profit": float(d.profit),
                              "exit_commission": float(d.commission),
                              "swap": float(d.swap), "exit_fee": float(d.fee)}
                    if all(math.isfinite(value) for value in values.values()):
                        components = values
                except (AttributeError, TypeError, ValueError, OverflowError):
                    pass
            # `profit` remains the numeric wire placeholder required by the
            # old Rust DTO. In strict mode ONLY later owned ClosedTrade receipts
            # carry realized PnL. These optional diagnostics are neither full
            # lifecycle NET (entry costs missing) nor a second booking source.
            return {
                "retcode": int(res.retcode), "order": int(res.order),
                "deal": int(res.deal), "position": int(p.ticket),
                "volume": float(res.volume), "price": float(res.price),
                "position_identifier": identifier if d is not None else 0,
                "profit": 0.0,
                "profit_basis": "receipt_only_no_realized_in_ack_v1",
                "ack_identity_complete": d is not None,
                "ack_deal_components": components,
                "comment": str(res.comment or ""),
            }
        profit = 0.0
        if res.deal:
            try:
                ds = mt5.history_deals_get(ticket=int(res.deal))
                if ds:
                    d = ds[0]
                    profit = float(d.profit) + float(getattr(d, "commission", 0.0)) \
                        + float(getattr(d, "swap", 0.0))
            except Exception:
                pass
        return {
            "retcode": int(res.retcode),
            "order": int(res.order),
            "deal": int(res.deal),
            "position": int(p.ticket),
            "volume": float(res.volume),
            "position_identifier": int(getattr(p, "identifier", 0) or 0),
            "price": float(res.price),
            "profit": profit,
            "comment": str(res.comment or ""),
        }

    def cmd_close_position(self, a):
        p = self._position(a.get("ticket"))
        return self._close(p.ticket, p.volume)

    def cmd_close_partial(self, a):
        ticket = int(a.get("ticket", 0))
        vol = float(a.get("volume", 0))
        p = self._position(ticket)
        if vol <= 0 or vol > p.volume + 1e-9:
            raise BrokerError(ERR_BAD_ARGS, "zły wolumen częściowy: %s z %s" % (vol, p.volume))
        return self._close(ticket, vol)

    def cmd_cancel_pending(self, a):
        ticket = int(a.get("ticket", 0))
        os_ = mt5.orders_get(ticket=ticket)
        if not os_:
            raise BrokerError(ERR_NO_TICKET, "brak zlecenia %s" % ticket)
        res = self._guarded_order_send({"action": TRADE_ACTION_REMOVE, "order": ticket})
        if res is None:
            code, desc = mt5.last_error()
            raise BrokerError(ERR_EXCEPTION, "order_send None: %s %s" % (code, desc))
        if res.retcode not in OK_CODES:
            raise BrokerError(res.retcode, str(res.comment or ""))
        return {"retcode": int(res.retcode), "order": ticket, "deal": 0, "position": 0,
                "volume": 0.0, "price": 0.0, "profit": 0.0}


def parse_args(argv=None):
    p = argparse.ArgumentParser(description="CONDUIT — sidecar MetaTrader 5")
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, required=True)
    p.add_argument("--symbol", default="XAUUSD")
    p.add_argument("--magic", type=int, default=770077)
    p.add_argument("--tick-ms", dest="tick_ms", type=int, default=50)
    p.add_argument("--deal-ms", dest="deal_ms", type=int, default=500)
    p.add_argument("--deviation", type=int, default=30)
    p.add_argument("--terminal", default=None, help="ścieżka do terminal64.exe")
    p.add_argument("--login", type=int, default=None)
    p.add_argument("--server", default=None)
    p.add_argument("--follow-terminal-account", action="store_true")
    p.add_argument("--allow-real-account", action="store_true")
    p.add_argument("--close-receipt-reconcile", action="store_true")
    p.add_argument("--closed-profit-net-costs", action="store_true")
    return p.parse_args(argv)


def pilnuj_rodzica():
    """Kończy proces, gdy Rust, który go uruchomił, przestaje istnieć.

    Pętla główna JEST jednowątkowa i tak zostaje — ten wątek nie dotyka
    pakietu `MetaTrader5`, tylko pyta system o istnienie procesu rodzica.

    Po co, skoro pętla i tak wychodzi na końcu strumienia: bo gdy terminal
    zamarza, `poll_tick` potrafi utknąć WEWNĄTRZ wywołania MT5 i nigdy nie
    dojść do miejsca, w którym wykryłoby zerwane gniazdo. Kontrola procesu
    rodzica zapobiega pozostawianiu kolejnych osieroconych helperów po restartach.
    """
    rodzic = os.getppid()
    if rodzic <= 0:
        return

    def zyje(pid):
        if os.name == "nt":
            import ctypes

            PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
            STILL_ACTIVE = 259
            k = ctypes.windll.kernel32
            h = k.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
            if not h:
                return False
            try:
                kod = ctypes.c_ulong()
                if k.GetExitCodeProcess(h, ctypes.byref(kod)) == 0:
                    return False
                return kod.value == STILL_ACTIVE
            finally:
                k.CloseHandle(h)
        try:
            os.kill(pid, 0)
            return True
        except OSError:
            return False

    while True:
        time.sleep(5.0)
        if not zyje(rodzic):
            log("Rust (PID %d) zniknął — kończę, żeby nie zostać sierotą" % rodzic)
            try:
                mt5.shutdown()
            except Exception:
                pass
            # `os._exit`, a nie `sys.exit`: jesteśmy w wątku pobocznym,
            # a pętla główna może wisieć w wywołaniu MT5, którego nic nie przerwie.
            os._exit(0)


def main():
    args = parse_args()
    s = Sidecar(args)
    threading.Thread(target=pilnuj_rodzica, daemon=True).start()
    try:
        s.run()
    except KeyboardInterrupt:
        pass
    except Exception:
        log(traceback.format_exc())
        return 1
    finally:
        try:
            mt5.shutdown()
        except Exception:
            pass
        if s.sock:
            try:
                s.sock.close()
            except OSError:
                pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
