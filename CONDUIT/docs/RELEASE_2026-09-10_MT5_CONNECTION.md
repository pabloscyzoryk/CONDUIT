# MT5 connection update — 2026-09-10

The bridge now becomes connected only after Python has successfully initialized
MetaTrader 5. Opening the local TCP socket alone no longer enables account queries
or trading requests. A failed initialization reaches the application with its
stage, error code and diagnostic text instead of being reduced to a generic
disconnection.

Startup has a separate readiness deadline. The normal RPC and keepalive limits
apply after initialization, so a valid slow SDK startup is not interrupted by the
shorter deadlines used during an established session. The application retains its
automatic retry loop when the terminal is unavailable.

On Windows, terminal discovery uses native limited-query process handles and the
bot's Windows session. A terminal in another RDP session does not block discovery.
Multiple matching processes remain ambiguous even when they run the same file;
an explicit path selects a terminal only when it identifies one running instance.
Both the live bridge and the read-only broker-history worker use this discovery.

The executable path is passed as the first positional argument to
`MetaTrader5.initialize`, following the
[MetaQuotes API contract](https://www.mql5.com/en/docs/python_metatrader5/mt5initialize_py).
Follow mode continues to use the terminal's current account without sending saved
login credentials. Vantage uses `XAUUSD`; PUPrime uses `XAUUSD.s`.

Validation covers native read-only Windows discovery, the production Python loop
with an offline SDK stub, and real local TCP transport. Cases include both startup
orders, delayed initialization, initialization failures and requests attempted
before readiness. These tests do not place orders or establish a real broker
connection; successful connection on a particular VPS still requires running the
updated package there.

GOD-X7 remains the selected strategy. This update changes connection handling,
diagnostics and their Polish/English translations; it does not change strategy,
signal parsing, risk settings or backtest arithmetic.
