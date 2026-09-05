//+------------------------------------------------------------------+
//|  ConduitBridge.mq5 — most CONDUIT wewnątrz terminala MetaTrader 5 |
//|                                                                  |
//|  SZKIC / PUNKT STARTOWY. Docelowa ścieżka transportu, alternatywa |
//|  dla sidecara w Pythonie.                                        |
//+------------------------------------------------------------------+
//
//  PO CO TO JEST
//  -------------
//  Sidecar w Pythonie rozmawia z terminalem po IPC pakietu MetaTrader5.
//  To działa, ale każde wywołanie to podróż przez granicę procesu, a strumień
//  ticków jest ODPYTYWANY (`symbol_info_tick` w pętli) — czyli z definicji
//  spóźniony o pół okresu odpytywania i gubiący ticki między odczytami.
//
//  Expert Advisor działa W ŚRODKU terminala. Dostaje `OnTick()` w chwili
//  nadejścia kwotowania i `OnTradeTransaction()` w chwili zawarcia transakcji.
//  Nic nie jest odpytywane, nic się nie gubi.
//
//  PROTOKÓŁ — DOKŁADNIE TEN SAM, CO W SIDECARZE
//  --------------------------------------------
//  Jedna linia = jeden dokument JSON, zakończona `\n`, kodowanie UTF-8.
//  Dzięki temu strona Rusta (`crates/mt5/src/proto.rs`) nie wymaga ŻADNEJ
//  zmiany — podmienia się tylko transport.
//
//  KIERUNEK POŁĄCZENIA
//  -------------------
//  MQL5 potrafi być wyłącznie KLIENTEM nazwanego potoku (`FileOpen` na ścieżce
//  `\\.\pipe\...`). Serwer potoku musi utworzyć Rust — czyli tak samo jak przy
//  TCP: Rust nasłuchuje, terminal się dobija. To nie jest ograniczenie, tylko
//  wygodny zbieg okoliczności: nadzór nad połączeniem zostaje po stronie
//  Rusta, w jednym miejscu dla obu transportów.
//
//  JAK UŻYĆ
//  --------
//   1. Skopiuj plik do  <katalog danych MT5>\MQL5\Experts\ConduitBridge.mq5
//      (Plik → Otwórz katalog danych).
//   2. Skompiluj w MetaEditor (F7).
//   3. Narzędzia → Opcje → Doradcy: zaznacz „Zezwalaj na handel algorytmiczny".
//   4. Po stronie Rusta utwórz serwer nazwanego potoku o nazwie z parametru
//      InpPipeName (patrz niżej — potrzebny mały wariant `Transport`
//      na `\\.\pipe\` zamiast TCP; reszta kodu bez zmian).
//   5. Wrzuć eksperta na wykres XAUUSD.
//
//  CZEGO TU JESZCZE NIE MA (świadomie — to szkic)
//  ---------------------------------------------
//   * pełnego parsera JSON: poniżej jest wyciąganie pól „na skróty", które
//     wystarcza dla ramek generowanych przez nasz Rust, ale nie jest odporne
//     na dowolny JSON (np. zagnieżdżone obiekty, znaki ucieczki w łańcuchach);
//   * obsługi wielu symboli naraz;
//   * `close_partial`, `modify_pending` — schemat jest identyczny jak przy
//     `close_position` / `modify_position`, dopisanie to kilkanaście linii;
//   * dobierania trybu wypełnienia w pętli po odmowie 10030 (w sidecarze jest).
//
//+------------------------------------------------------------------+
#property copyright "CONDUIT"
#property version   "0.2"
#property strict

input string InpPipeName   = "CONDUIT_MT5";  // nazwa potoku (bez \\.\pipe\)
input long   InpMagic      = 770077;         // magic bota
input int    InpDeviation  = 30;             // dopuszczalne odchylenie [pkt]
input int    InpTimerMs    = 10;             // co ile czytać potok [ms]

#define PROTO_VERSION 1

int    g_pipe = INVALID_HANDLE;
string g_rx   = "";     // bufor niedokończonej linii
string g_sym  = "";

//+------------------------------------------------------------------+
//| Pomocnicze: składanie JSON-a                                     |
//+------------------------------------------------------------------+
string JQ(string s)                    { return "\"" + s + "\""; }
string JKV(string k, string v)         { return JQ(k) + ":" + v; }
string JKS(string k, string v)         { return JQ(k) + ":" + JQ(v); }
string JKI(string k, long v)           { return JQ(k) + ":" + (string)v; }
string JKD(string k, double v, int d)  { return JQ(k) + ":" + DoubleToString(v, d); }

//+------------------------------------------------------------------+
//| Wyciągnięcie pola z płaskiego JSON-a.                            |
//| Uproszczenie: zakłada ramki generowane przez naszego Rusta.      |
//+------------------------------------------------------------------+
string JGet(string json, string key)
{
   string pat = "\"" + key + "\":";
   int p = StringFind(json, pat);
   if(p < 0) return "";
   p += StringLen(pat);
   while(p < StringLen(json) && StringGetCharacter(json, p) == ' ') p++;
   if(p >= StringLen(json)) return "";

   ushort c = StringGetCharacter(json, p);
   if(c == '"')                       // łańcuch
   {
      int e = StringFind(json, "\"", p + 1);
      if(e < 0) return "";
      return StringSubstr(json, p + 1, e - p - 1);
   }
   int e = p;                          // liczba / null / true / false
   while(e < StringLen(json))
   {
      ushort d = StringGetCharacter(json, e);
      if(d == ',' || d == '}' || d == ']') break;
      e++;
   }
   string v = StringSubstr(json, p, e - p);
   StringTrimLeft(v); StringTrimRight(v);
   return v;
}

double JGetD(string json, string key, double def)
{
   string v = JGet(json, key);
   if(v == "" || v == "null") return def;
   return StringToDouble(v);
}

long JGetI(string json, string key, long def)
{
   string v = JGet(json, key);
   if(v == "" || v == "null") return def;
   return (long)StringToInteger(v);
}

bool JHas(string json, string key)
{
   string v = JGet(json, key);
   return (v != "" && v != "null");
}

//+------------------------------------------------------------------+
//| Potok                                                            |
//+------------------------------------------------------------------+
bool PipeConnect()
{
   if(g_pipe != INVALID_HANDLE) return true;
   string path = "\\\\.\\pipe\\" + InpPipeName;
   // FILE_BIN, bo linie sklejamy sami; FILE_ANSI nie tknie znaków UTF-8
   g_pipe = FileOpen(path, FILE_READ | FILE_WRITE | FILE_BIN);
   if(g_pipe == INVALID_HANDLE) return false;
   PrintFormat("CONDUIT: potok %s otwarty", path);
   Send("{" + JKS("ev","hello") + "," + JKI("proto",PROTO_VERSION) + ","
        + JKS("sidecar","ConduitBridge.mq5") + ","
        + JKS("mt5_version", (string)TerminalInfoInteger(TERMINAL_BUILD)) + "}");
   return true;
}

void PipeClose()
{
   if(g_pipe != INVALID_HANDLE) { FileClose(g_pipe); g_pipe = INVALID_HANDLE; }
   g_rx = "";
}

void Send(string line)
{
   if(g_pipe == INVALID_HANDLE) return;
   string s = line + "\n";
   uchar buf[];
   int n = StringToCharArray(s, buf, 0, WHOLE_ARRAY, CP_UTF8);
   if(n > 0) n--;                       // StringToCharArray dokleja \0
   if(FileWriteArray(g_pipe, buf, 0, n) != n)
   {
      Print("CONDUIT: zapis do potoku nieudany — rozłączam");
      PipeClose();
      return;
   }
   FileFlush(g_pipe);
}

void ReplyOk(long id, string result)
{
   Send("{" + JKI("id",id) + "," + JKV("ok","true") + "," + JKV("result",result) + "}");
}

void ReplyErr(long id, long code, string msg)
{
   Send("{" + JKI("id",id) + "," + JKV("ok","false") + ","
        + JKV("error","{" + JKI("code",code) + "," + JKS("msg",msg) + "}") + "}");
}

//+------------------------------------------------------------------+
//| Cykl życia                                                       |
//+------------------------------------------------------------------+
int OnInit()
{
   g_sym = _Symbol;
   EventSetMillisecondTimer(InpTimerMs);
   PipeConnect();
   return INIT_SUCCEEDED;
}

void OnDeinit(const int reason)
{
   EventKillTimer();
   PipeClose();
}

//+------------------------------------------------------------------+
//| Tick — wypychany NATYCHMIAST, bez odpytywania                    |
//+------------------------------------------------------------------+
void OnTick()
{
   if(g_pipe == INVALID_HANDLE) return;
   MqlTick t;
   if(!SymbolInfoTick(g_sym, t)) return;
   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   Send("{" + JKS("ev","tick") + "," + JKI("ts",(long)t.time_msc) + ","
        + JKD("bid",t.bid,d) + "," + JKD("ask",t.ask,d) + "}");
}

//+------------------------------------------------------------------+
//| Zamknięcia — ze zdarzenia, nie z przeglądania historii           |
//+------------------------------------------------------------------+
void OnTradeTransaction(const MqlTradeTransaction &trans,
                        const MqlTradeRequest &request,
                        const MqlTradeResult &result)
{
   if(g_pipe == INVALID_HANDLE) return;
   if(trans.type != TRADE_TRANSACTION_DEAL_ADD) return;
   if(!HistoryDealSelect(trans.deal)) return;
   if(HistoryDealGetInteger(trans.deal, DEAL_MAGIC) != InpMagic) return;

   long entry = HistoryDealGetInteger(trans.deal, DEAL_ENTRY);
   if(entry != DEAL_ENTRY_OUT && entry != DEAL_ENTRY_OUT_BY) return;

   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   Send("{" + JKS("ev","closed") + ","
      + JKI("deal",      (long)trans.deal) + ","
      + JKI("position",  HistoryDealGetInteger(trans.deal, DEAL_POSITION_ID)) + ","
      + JKI("deal_type", HistoryDealGetInteger(trans.deal, DEAL_TYPE)) + ","
      + JKD("volume",    HistoryDealGetDouble(trans.deal, DEAL_VOLUME), 2) + ","
      + JKD("price",     HistoryDealGetDouble(trans.deal, DEAL_PRICE), d) + ","
      + JKI("time_msc",  HistoryDealGetInteger(trans.deal, DEAL_TIME_MSC)) + ","
      + JKD("profit",    HistoryDealGetDouble(trans.deal, DEAL_PROFIT), 2) + ","
      + JKD("commission",HistoryDealGetDouble(trans.deal, DEAL_COMMISSION), 2) + ","
      + JKD("swap",      HistoryDealGetDouble(trans.deal, DEAL_SWAP), 2) + ","
      + JKI("reason",    HistoryDealGetInteger(trans.deal, DEAL_REASON)) + ","
      + JKI("magic",     InpMagic) + "}");
}

//+------------------------------------------------------------------+
//| Timer — czytanie żądań i wznawianie połączenia                   |
//+------------------------------------------------------------------+
void OnTimer()
{
   if(g_pipe == INVALID_HANDLE) { PipeConnect(); return; }

   while(!FileIsEnding(g_pipe))
   {
      ulong avail = FileSize(g_pipe) - FileTell(g_pipe);
      if(avail <= 0) break;
      uchar buf[];
      int n = FileReadArray(g_pipe, buf, 0, (int)MathMin(avail, 65536));
      if(n <= 0) break;
      g_rx += CharArrayToString(buf, 0, n, CP_UTF8);
   }

   int nl;
   while((nl = StringFind(g_rx, "\n")) >= 0)
   {
      string line = StringSubstr(g_rx, 0, nl);
      g_rx = StringSubstr(g_rx, nl + 1);
      StringTrimLeft(line); StringTrimRight(line);
      if(line != "") Dispatch(line);
   }
}

//+------------------------------------------------------------------+
//| Obsługa żądań                                                    |
//+------------------------------------------------------------------+
void Dispatch(string req)
{
   long   id  = JGetI(req, "id", 0);
   string cmd = JGet(req, "cmd");
   if(id == 0 || cmd == "") return;

   if(cmd == "ping")           { ReplyOk(id, "{\"pong\":true}");        return; }
   if(cmd == "symbol_info")    { CmdSymbolInfo(id);                     return; }
   if(cmd == "account")        { CmdAccount(id);                        return; }
   if(cmd == "quote")          { CmdQuote(id);                          return; }
   if(cmd == "positions")      { CmdPositions(id);                      return; }
   if(cmd == "orders")         { CmdOrders(id);                         return; }
   if(cmd == "subscribe_ticks"){ ReplyOk(id, "{" + JKS("symbol",g_sym) + "}"); return; }
   if(cmd == "open_market")    { CmdOpenMarket(id, req);                return; }
   if(cmd == "place_pending")  { CmdPlacePending(id, req);              return; }
   if(cmd == "modify_position"){ CmdModifyPosition(id, req);            return; }
   if(cmd == "close_position") { CmdClosePosition(id, req);             return; }
   if(cmd == "cancel_pending") { CmdCancelPending(id, req);             return; }
   if(cmd == "shutdown")       { ReplyOk(id, "null"); PipeClose();      return; }

   ReplyErr(id, -2, "nieznana komenda: " + cmd);   // -2 = UNKNOWN_CMD
}

void CmdSymbolInfo(long id)
{
   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   string r = "{"
      + JKS("symbol", g_sym) + ","
      + JKI("digits", d) + ","
      + JKD("point", SymbolInfoDouble(g_sym, SYMBOL_POINT), 8) + ","
      + JKI("stops_level_points", SymbolInfoInteger(g_sym, SYMBOL_TRADE_STOPS_LEVEL)) + ","
      + JKI("freeze_level_points", SymbolInfoInteger(g_sym, SYMBOL_TRADE_FREEZE_LEVEL)) + ","
      + JKD("volume_min", SymbolInfoDouble(g_sym, SYMBOL_VOLUME_MIN), 4) + ","
      + JKD("volume_max", SymbolInfoDouble(g_sym, SYMBOL_VOLUME_MAX), 4) + ","
      + JKD("volume_step", SymbolInfoDouble(g_sym, SYMBOL_VOLUME_STEP), 4) + ","
      + JKD("contract_size", SymbolInfoDouble(g_sym, SYMBOL_TRADE_CONTRACT_SIZE), 4) + ","
      + JKI("filling_mask", SymbolInfoInteger(g_sym, SYMBOL_FILLING_MODE)) + ","
      + JKI("filling_market", (long)PickFilling()) + ","
      + JKI("filling_pending", (long)ORDER_FILLING_RETURN) + ","
      + JKI("trade_mode", SymbolInfoInteger(g_sym, SYMBOL_TRADE_MODE))
      + "}";
   ReplyOk(id, r);
}

ENUM_ORDER_TYPE_FILLING PickFilling()
{
   long mask = SymbolInfoInteger(g_sym, SYMBOL_FILLING_MODE);
   if((mask & SYMBOL_FILLING_IOC) != 0) return ORDER_FILLING_IOC;
   if((mask & SYMBOL_FILLING_FOK) != 0) return ORDER_FILLING_FOK;
   return ORDER_FILLING_RETURN;
}

void CmdAccount(long id)
{
   string r = "{"
      + JKD("balance",     AccountInfoDouble(ACCOUNT_BALANCE), 2) + ","
      + JKD("equity",      AccountInfoDouble(ACCOUNT_EQUITY), 2) + ","
      + JKD("margin",      AccountInfoDouble(ACCOUNT_MARGIN), 2) + ","
      + JKD("margin_free", AccountInfoDouble(ACCOUNT_MARGIN_FREE), 2) + ","
      + JKI("leverage",    AccountInfoInteger(ACCOUNT_LEVERAGE)) + ","
      + JKS("currency",    AccountInfoString(ACCOUNT_CURRENCY))
      + "}";
   ReplyOk(id, r);
}

void CmdQuote(long id)
{
   MqlTick t;
   if(!SymbolInfoTick(g_sym, t)) { ReplyErr(id, -1, "brak kwotowania"); return; }
   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   ReplyOk(id, "{" + JKI("ts",(long)t.time_msc) + ","
                   + JKD("bid",t.bid,d) + "," + JKD("ask",t.ask,d) + "}");
}

void CmdPositions(long id)
{
   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   string arr = "[";
   int k = 0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
   {
      ulong tk = PositionGetTicket(i);
      if(tk == 0 || !PositionSelectByTicket(tk)) continue;
      if(PositionGetString(POSITION_SYMBOL) != g_sym) continue;
      if(k++ > 0) arr += ",";
      arr += "{"
         + JKI("ticket",    (long)tk) + ","
         + JKI("kind",      PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY ? 0 : 1) + ","
         + JKD("volume",    PositionGetDouble(POSITION_VOLUME), 2) + ","
         + JKD("price_open",PositionGetDouble(POSITION_PRICE_OPEN), d) + ","
         + JKI("time_msc",  PositionGetInteger(POSITION_TIME_MSC)) + ","
         + JKD("sl",        PositionGetDouble(POSITION_SL), d) + ","
         + JKD("tp",        PositionGetDouble(POSITION_TP), d) + ","
         + JKD("profit",    PositionGetDouble(POSITION_PROFIT), 2) + ","
         + JKI("magic",     PositionGetInteger(POSITION_MAGIC)) + ","
         + JKS("comment",   PositionGetString(POSITION_COMMENT)) + ","
         + JKS("symbol",    PositionGetString(POSITION_SYMBOL))
         + "}";
   }
   ReplyOk(id, arr + "]");
}

void CmdOrders(long id)
{
   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   string arr = "[";
   int k = 0;
   for(int i = OrdersTotal() - 1; i >= 0; i--)
   {
      ulong tk = OrderGetTicket(i);
      if(tk == 0 || !OrderSelect(tk)) continue;
      if(OrderGetString(ORDER_SYMBOL) != g_sym) continue;
      if(k++ > 0) arr += ",";
      arr += "{"
         + JKI("ticket",    (long)tk) + ","
         + JKI("kind",      OrderGetInteger(ORDER_TYPE)) + ","
         + JKD("volume",    OrderGetDouble(ORDER_VOLUME_CURRENT), 2) + ","
         + JKD("price_open",OrderGetDouble(ORDER_PRICE_OPEN), d) + ","
         + JKI("time_msc",  OrderGetInteger(ORDER_TIME_SETUP_MSC)) + ","
         + JKD("sl",        OrderGetDouble(ORDER_SL), d) + ","
         + JKD("tp",        OrderGetDouble(ORDER_TP), d) + ","
         + JKI("magic",     OrderGetInteger(ORDER_MAGIC)) + ","
         + JKS("comment",   OrderGetString(ORDER_COMMENT)) + ","
         + JKS("symbol",    OrderGetString(ORDER_SYMBOL))
         + "}";
   }
   ReplyOk(id, arr + "]");
}

string SendResultJson(MqlTradeResult &res, long position, double profit)
{
   int d = (int)SymbolInfoInteger(g_sym, SYMBOL_DIGITS);
   return "{"
      + JKI("retcode",  res.retcode) + ","
      + JKI("order",    (long)res.order) + ","
      + JKI("deal",     (long)res.deal) + ","
      + JKI("position", position) + ","
      + JKD("volume",   res.volume, 2) + ","
      + JKD("price",    res.price, d) + ","
      + JKD("profit",   profit, 2) + ","
      + JKS("comment",  res.comment)
      + "}";
}

bool Ok(uint rc) { return rc == TRADE_RETCODE_DONE || rc == TRADE_RETCODE_PLACED
                       || rc == TRADE_RETCODE_DONE_PARTIAL; }

void CmdOpenMarket(long id, string req)
{
   string side = JGet(req, "side");
   MqlTick t;
   if(!SymbolInfoTick(g_sym, t)) { ReplyErr(id, -1, "brak kwotowania"); return; }

   MqlTradeRequest  r; ZeroMemory(r);
   MqlTradeResult   s; ZeroMemory(s);
   r.action       = TRADE_ACTION_DEAL;
   r.symbol       = g_sym;
   r.volume       = JGetD(req, "volume", 0);
   r.type         = (side == "buy") ? ORDER_TYPE_BUY : ORDER_TYPE_SELL;
   r.price        = (side == "buy") ? t.ask : t.bid;
   r.deviation    = InpDeviation;
   r.magic        = InpMagic;
   r.type_filling = PickFilling();
   r.type_time    = ORDER_TIME_GTC;
   r.comment      = JGet(req, "comment");
   if(JHas(req, "sl")) r.sl = JGetD(req, "sl", 0);
   if(JHas(req, "tp")) r.tp = JGetD(req, "tp", 0);

   if(!OrderSend(r, s) || !Ok(s.retcode)) { ReplyErr(id, s.retcode, s.comment); return; }

   long pos = 0;
   if(s.deal != 0 && HistoryDealSelect(s.deal))
      pos = HistoryDealGetInteger(s.deal, DEAL_POSITION_ID);
   if(pos == 0) pos = (long)s.order;
   ReplyOk(id, SendResultJson(s, pos, 0.0));
}

void CmdPlacePending(long id, string req)
{
   MqlTradeRequest  r; ZeroMemory(r);
   MqlTradeResult   s; ZeroMemory(s);
   r.action       = TRADE_ACTION_PENDING;
   r.symbol       = g_sym;
   r.volume       = JGetD(req, "volume", 0);
   r.type         = (ENUM_ORDER_TYPE)JGetI(req, "kind", 2);
   r.price        = JGetD(req, "price", 0);
   r.magic        = InpMagic;
   r.type_filling = ORDER_FILLING_RETURN;
   r.type_time    = ORDER_TIME_GTC;
   r.comment      = JGet(req, "comment");
   if(JHas(req, "sl")) r.sl = JGetD(req, "sl", 0);
   if(JHas(req, "tp")) r.tp = JGetD(req, "tp", 0);

   if(!OrderSend(r, s) || !Ok(s.retcode)) { ReplyErr(id, s.retcode, s.comment); return; }
   ReplyOk(id, SendResultJson(s, 0, 0.0));
}

void CmdModifyPosition(long id, string req)
{
   ulong tk = (ulong)JGetI(req, "ticket", 0);
   if(!PositionSelectByTicket(tk)) { ReplyErr(id, -4, "brak pozycji"); return; }

   MqlTradeRequest  r; ZeroMemory(r);
   MqlTradeResult   s; ZeroMemory(s);
   r.action   = TRADE_ACTION_SLTP;
   r.symbol   = PositionGetString(POSITION_SYMBOL);
   r.position = tk;
   r.sl       = JHas(req, "sl") ? JGetD(req, "sl", 0) : 0.0;
   r.tp       = JHas(req, "tp") ? JGetD(req, "tp", 0) : 0.0;

   if(!OrderSend(r, s) || !Ok(s.retcode)) { ReplyErr(id, s.retcode, s.comment); return; }
   ReplyOk(id, SendResultJson(s, (long)tk, 0.0));
}

void CmdClosePosition(long id, string req)
{
   ulong tk = (ulong)JGetI(req, "ticket", 0);
   if(!PositionSelectByTicket(tk)) { ReplyErr(id, -4, "brak pozycji"); return; }

   bool is_buy = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY);
   string sym  = PositionGetString(POSITION_SYMBOL);
   MqlTick t;
   if(!SymbolInfoTick(sym, t)) { ReplyErr(id, -1, "brak kwotowania"); return; }

   MqlTradeRequest  r; ZeroMemory(r);
   MqlTradeResult   s; ZeroMemory(s);
   r.action       = TRADE_ACTION_DEAL;
   r.symbol       = sym;
   r.position     = tk;
   r.volume       = PositionGetDouble(POSITION_VOLUME);
   // pozycję BUY zamyka zlecenie SELL wykonane po BID — i odwrotnie
   r.type         = is_buy ? ORDER_TYPE_SELL : ORDER_TYPE_BUY;
   r.price        = is_buy ? t.bid : t.ask;
   r.deviation    = InpDeviation;
   r.magic        = InpMagic;
   r.type_filling = PickFilling();
   r.type_time    = ORDER_TIME_GTC;

   if(!OrderSend(r, s) || !Ok(s.retcode)) { ReplyErr(id, s.retcode, s.comment); return; }

   double profit = 0.0;
   if(s.deal != 0 && HistoryDealSelect(s.deal))
      profit = HistoryDealGetDouble(s.deal, DEAL_PROFIT)
             + HistoryDealGetDouble(s.deal, DEAL_COMMISSION)
             + HistoryDealGetDouble(s.deal, DEAL_SWAP);
   ReplyOk(id, SendResultJson(s, (long)tk, profit));
}

void CmdCancelPending(long id, string req)
{
   ulong tk = (ulong)JGetI(req, "ticket", 0);
   MqlTradeRequest  r; ZeroMemory(r);
   MqlTradeResult   s; ZeroMemory(s);
   r.action = TRADE_ACTION_REMOVE;
   r.order  = tk;
   if(!OrderSend(r, s) || !Ok(s.retcode)) { ReplyErr(id, s.retcode, s.comment); return; }
   ReplyOk(id, SendResultJson(s, 0, 0.0));
}
//+------------------------------------------------------------------+
