#property copyright "CONDUIT"
#property version "1.00"
#property strict

input string In_OutputFile = "conduit_tick_dump.csv";
int g_file = INVALID_HANDLE;
long g_count = 0;

int OnInit()
  {
   if(!MQLInfoInteger(MQL_TESTER))
     {
      Print("CONDUIT_TICK_DUMP is restricted to Strategy Tester.");
      return INIT_FAILED;
     }
   g_file=FileOpen(In_OutputFile,FILE_WRITE|FILE_TXT|FILE_ANSI|FILE_COMMON);
   if(g_file==INVALID_HANDLE) return INIT_FAILED;
   FileWrite(g_file,"server_time_ms;bid;ask");
   PrintFormat("BROKER_SPEC symbol=%s digits=%d point=%.8f tick_size=%.8f "
               "tick_value=%.8f contract=%.2f stops=%.8f freeze=%.8f "
               "vol_min=%.4f vol_step=%.4f vol_max=%.4f swap_mode=%d "
               "swap_long=%.8f swap_short=%.8f swap3day=%d leverage=%d hedging=%d",
               _Symbol,(int)SymbolInfoInteger(_Symbol,SYMBOL_DIGITS),
               SymbolInfoDouble(_Symbol,SYMBOL_POINT),
               SymbolInfoDouble(_Symbol,SYMBOL_TRADE_TICK_SIZE),
               SymbolInfoDouble(_Symbol,SYMBOL_TRADE_TICK_VALUE),
               SymbolInfoDouble(_Symbol,SYMBOL_TRADE_CONTRACT_SIZE),
               SymbolInfoInteger(_Symbol,SYMBOL_TRADE_STOPS_LEVEL)*SymbolInfoDouble(_Symbol,SYMBOL_POINT),
               SymbolInfoInteger(_Symbol,SYMBOL_TRADE_FREEZE_LEVEL)*SymbolInfoDouble(_Symbol,SYMBOL_POINT),
               SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_MIN),
               SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_STEP),
               SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_MAX),
               (int)SymbolInfoInteger(_Symbol,SYMBOL_SWAP_MODE),
               SymbolInfoDouble(_Symbol,SYMBOL_SWAP_LONG),
               SymbolInfoDouble(_Symbol,SYMBOL_SWAP_SHORT),
               (int)SymbolInfoInteger(_Symbol,SYMBOL_SWAP_ROLLOVER3DAYS),
               (int)AccountInfoInteger(ACCOUNT_LEVERAGE),
               (int)(AccountInfoInteger(ACCOUNT_MARGIN_MODE)==ACCOUNT_MARGIN_MODE_RETAIL_HEDGING));
   return INIT_SUCCEEDED;
  }

void OnTick()
  {
   MqlTick tick;
   if(!SymbolInfoTick(_Symbol,tick)) return;
   FileWrite(g_file,IntegerToString(tick.time_msc)+";"+
             DoubleToString(tick.bid,8)+";"+DoubleToString(tick.ask,8));
   g_count++;
  }

void OnDeinit(const int reason)
  {
   if(g_file!=INVALID_HANDLE) FileClose(g_file);
   PrintFormat("CONDUIT_TICK_DUMP complete: ticks=%I64d reason=%d",g_count,reason);
  }
