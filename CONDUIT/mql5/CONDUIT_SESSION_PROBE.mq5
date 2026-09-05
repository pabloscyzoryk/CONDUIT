#property strict
// Offline diagnostic: real tick/session execution boundary, no strategy inputs.
input string In_SetupAt = "2026.07.27 23:57:55";
input string In_StopAt = "2026.07.28 01:01:01";
input double In_CrossLevel = 4078.0;
input double In_IdleLimit = 4000.0;
bool setup=false, client_probe=false;
long setup_at=0, stop_at=0;
ulong sell_position=0, buy_position=0, idle_order=0;
ENUM_ORDER_TYPE_FILLING filling=ORDER_FILLING_FOK;
const ulong MAGIC=970083;

ulong Send(ENUM_ORDER_TYPE type,double sl,double tp,string label)
  {
   MqlTick q; SymbolInfoTick(_Symbol,q);
   MqlTradeRequest r={}; MqlTradeResult x={};
   r.symbol=_Symbol;r.magic=MAGIC;r.volume=0.01;r.type=type;r.sl=sl;r.tp=tp;
   r.action=type<=ORDER_TYPE_SELL ? TRADE_ACTION_DEAL : TRADE_ACTION_PENDING;
   r.type_filling=r.action==TRADE_ACTION_DEAL ? filling : ORDER_FILLING_RETURN;
   r.type_time=ORDER_TIME_GTC;r.comment=label;
   r.price=type==ORDER_TYPE_BUY ? q.ask : type==ORDER_TYPE_SELL ? q.bid
           : label=="idle" ? In_IdleLimit : In_CrossLevel;
   bool sent=OrderSend(r,x);
   PrintFormat("SESSION_PROBE action=%s ts=%I64d sent=%d retcode=%u ticket=%I64u bid=%.2f ask=%.2f",
               label,q.time_msc,sent,x.retcode,x.order,q.bid,q.ask);
   return sent && (x.retcode==TRADE_RETCODE_DONE || x.retcode==TRADE_RETCODE_PLACED) ? x.order : 0;
  }
void Request(ENUM_TRADE_REQUEST_ACTIONS action,ulong ticket,string label)
  {
   MqlTick q;SymbolInfoTick(_Symbol,q);
   MqlTradeRequest r={};MqlTradeResult x={};r.action=action;r.symbol=_Symbol;r.magic=MAGIC;
   if(action==TRADE_ACTION_REMOVE)r.order=ticket;
   else if(action==TRADE_ACTION_MODIFY){r.order=ticket;r.price=In_IdleLimit-1.0;r.type_time=ORDER_TIME_GTC;}
   else if(action==TRADE_ACTION_SLTP){r.position=ticket;r.sl=label=="closed_modify_position_valid" ? q.ask+1.0 : In_CrossLevel+1.0;}
   else {r.position=ticket;r.volume=0.01;r.type=ORDER_TYPE_BUY;r.price=q.ask;r.type_filling=filling;}
   bool sent=OrderSend(r,x);
   PrintFormat("SESSION_PROBE action=%s ts=%I64d sent=%d retcode=%u ticket=%I64u",
               label,q.time_msc,sent,x.retcode,ticket);
  }
int OnInit()
  {
   if(!MQLInfoInteger(MQL_TESTER))return INIT_FAILED;
   if(AccountInfoInteger(ACCOUNT_MARGIN_MODE)!=ACCOUNT_MARGIN_MODE_RETAIL_HEDGING)return INIT_FAILED;
   setup_at=(long)StringToTime(In_SetupAt)*1000;stop_at=(long)StringToTime(In_StopAt)*1000;
   if(setup_at<=0 || stop_at<=setup_at)return INIT_PARAMETERS_INCORRECT;
   long mask=SymbolInfoInteger(_Symbol,SYMBOL_FILLING_MODE);
   if((mask&SYMBOL_FILLING_FOK)==0 && (mask&SYMBOL_FILLING_IOC)!=0)filling=ORDER_FILLING_IOC;
   return INIT_SUCCEEDED;
  }
void OnTick()
  {
   MqlTick q;SymbolInfoTick(_Symbol,q);if(q.time_msc<setup_at)return;
   if(!setup)
     {
      setup=true;
      sell_position=Send(ORDER_TYPE_SELL,In_CrossLevel,0,"sell_sl");
      buy_position=Send(ORDER_TYPE_BUY,0,In_CrossLevel,"buy_tp");
      Send(ORDER_TYPE_BUY_STOP,0,0,"buy_stop");
      Send(ORDER_TYPE_SELL_LIMIT,0,0,"sell_limit");
      idle_order=Send(ORDER_TYPE_BUY_LIMIT,0,0,"idle");
     }
   MqlDateTime t;TimeToStruct(q.time,t);
   if(t.hour==1 && t.min<=1)
     {
      PrintFormat("SESSION_PROBE action=observation ts=%I64d bid=%.2f ask=%.2f positions=%d orders=%d sell_alive=%d buy_alive=%d",
                  q.time_msc,q.bid,q.ask,PositionsTotal(),OrdersTotal(),PositionSelectByTicket(sell_position),PositionSelectByTicket(buy_position));
      if(!client_probe && t.min==0)
        {
         client_probe=true;Send(ORDER_TYPE_BUY,0,0,"closed_market");
         Send(ORDER_TYPE_BUY_LIMIT,0,0,"closed_pending");
         Request(TRADE_ACTION_SLTP,sell_position,"closed_modify_position");
         Request(TRADE_ACTION_SLTP,sell_position,"closed_modify_position_valid");
         Request(TRADE_ACTION_MODIFY,idle_order,"closed_modify_pending");
         Request(TRADE_ACTION_DEAL,sell_position,"closed_close");
         Request(TRADE_ACTION_REMOVE,idle_order,"closed_cancel");
        }
     }
   if(q.time_msc>=stop_at)TesterStop();
  }
void OnDeinit(const int reason)
  {
   HistorySelect(0,TimeCurrent()+86400);
   for(int i=0;i<HistoryDealsTotal();i++)
     {
      ulong d=HistoryDealGetTicket(i);if(HistoryDealGetInteger(d,DEAL_MAGIC)!=MAGIC)continue;
      PrintFormat("SESSION_PROBE action=deal ts=%I64d deal=%I64u position=%I64d entry=%d reason=%d price=%.2f volume=%.2f",
                  HistoryDealGetInteger(d,DEAL_TIME_MSC),d,HistoryDealGetInteger(d,DEAL_POSITION_ID),
                  (int)HistoryDealGetInteger(d,DEAL_ENTRY),(int)HistoryDealGetInteger(d,DEAL_REASON),
                  HistoryDealGetDouble(d,DEAL_PRICE),HistoryDealGetDouble(d,DEAL_VOLUME));
     }
  }
