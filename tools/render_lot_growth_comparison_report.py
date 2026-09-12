"""Private full-owner comparison after frozen TRAIN/August qualification.

  python tools/render_lot_growth_comparison_report.py FULL_STUDY --output NEW.html

No backtest is launched, no finalist is selected, and no preset is promoted.
Every cap/deposit row describes a separate account, never a pooled return.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path

import prepare_lot_growth_comparison as comparison
import prepare_lot_growth_sweep as prep
import rank_lot_growth_sweep as rank
import validate_lot_growth_finalists as august

SCHEMA='conduit.sizing-full-owner-report.v1'


def optional_number(value):
    if value is not None and not august.number(value):
        raise ValueError('Invalid optional financial observation')
    return value


def source_statistics(metrics):
    """Accepted independent entry sources are not filled baskets or win rate."""
    f=metrics.get('stat_sygnalow',{}).get('lejek',{})
    if type(f.get('source_observation_version')) is not int or f['source_observation_version']!=1:
        return None
    fields=('sygnaly_wejsciowe','koszyki_sygnaly','unattributed_entry_outcomes')
    if any(type(f.get(k)) is not int or f[k]<0 for k in fields):
        return None
    if not isinstance(f.get('source_identity_semantics'),str) or not f['source_identity_semantics']:
        return None
    pct=f.get('accepted_entry_sources_pct')
    if pct is not None and not august.number(pct):
        raise ValueError('Invalid source acceptance statistic')
    return {'version':1,'observed':f['sygnaly_wejsciowe'],'accepted':f['koszyki_sygnaly'],
            'unattributed':f['unattributed_entry_outcomes'],'reported_accepted_pct':pct,
            'basis':f['source_identity_semantics'],'not_filled_baskets_or_win_rate':True}


def build_payload(study):
    root=Path(study).resolve()
    comparison.check(root)
    plan=prep.read(root/'PLAN.json');contract=plan['comparison_contract']
    freeze=comparison.frozen_selection(contract['final_selection_directory'])
    evidence=august.unique_pins(plan['inputs']+[prep.pin(root/'PLAN.json'),prep.pin(root/'PREPARATION_RECEIPT.json'),
        prep.pin(__file__),prep.pin(comparison.__file__),prep.pin(august.__file__)])
    rows=[];finalists={r['id'] for r in freeze['finalists']}
    train=freeze['train'];validation={r['id']:r for r in freeze['selection']['all_shortlisted']}
    for job in plan['jobs']:
        registry=contract['registry'][job['id']]
        described,pins=august.completed_job(root/'PLAN.json',plan,job,registry,
                                           {r['path']:r for r in plan['inputs']})
        summary,details=rank.verified_results(root,job['id'])
        evidence+=pins
        for name in sorted(registry):
            record=registry[name];d=details[name];row=described[name]
            config=prep.read(record['file']['path'])['settings'];control=record['control']
            daily_rdd=[optional_number(day.get('real_dd')) for day in d['dni']]
            row.update(key=job['id']+'/'+name,job=job['id'],cap=record['cap'],control=control,
                window=list(comparison.WINDOW),deposit=600,
                train_status='CONTROL' if control else 'FROZEN_TRAIN_SHORTLIST',
                august_status='CONTROL' if control else ('FROZEN_FINALIST' if name in finalists else 'NOT_A_FINALIST'),
                train=None if control else train['rows'][name],
                august=None if control else validation[name],
                source_statistics=source_statistics(summary[name]),days=d['dni'],
                sizing={key:config[key] for key in sorted(prep.NEW_FIELDS)},
                daily_balance_path=None,equity_path='daily_boundaries_only',
                end_balance=optional_number(summary[name].get('end_balance')),
                max_dd_abs=optional_number(summary[name].get('max_dd_abs')),
                max_daily_rdd_abs=max(daily_rdd) if daily_rdd and all(v is not None for v in daily_rdd) else None,
                config_pin=record['file'])
            ledger=Path(job['result_dir'])/(name+'_compound_transakcje.json')
            if ledger.exists():evidence.append(prep.pin(ledger))
            rows.append(row)
    expected=3*(len(contract['train_shortlist_ids'])+1)
    if len(rows)!=expected or len({r['key'] for r in rows})!=expected:
        raise ValueError('Incomplete full-owner comparison')
    payload={'schema':SCHEMA,'scope':'FULL_OWNER_COMPARISON_AFTER_AUGUST_FREEZE',
        'study_name':plan['name'],'window':list(comparison.WINDOW),'deposit':600,
        'caps':[cap for _,cap in comparison.CAPS],'rows':rows,'accounts':expected,
        'train_shortlist':contract['train_shortlist_ids'],'august_finalists':contract['august_finalist_ids'],
        'august_status':freeze['selection']['status'],'no_finalists':not bool(finalists),
        'used_to_select':False,'certified_god_x8':False,'independent_accounts':True,
        'no_untouched_holdout':True,'source_chronology':'publication_final_not_complete_live_edits',
        'plan':prep.pin(root/'PLAN.json'),'final_freeze':freeze['final_pins'],
        'evidence':august.unique_pins(evidence)}
    json.dumps(payload,allow_nan=False)
    for p in payload['evidence']:prep.verify_pin(p)
    return payload


def render(study,output):
    output=Path(output).resolve();receipt_path=output.with_suffix(output.suffix+'.receipt.json')
    if output.exists() or receipt_path.exists():raise FileExistsError(output)
    payload=build_payload(study)
    encoded=json.dumps(payload,ensure_ascii=False,allow_nan=False).replace('<','\\u003c')
    document=TEMPLATE.replace('__DATA__',encoded)
    output.parent.mkdir(parents=True,exist_ok=True)
    with output.open('x',encoding='utf-8') as f:f.write(document)
    receipt={'schema':SCHEMA,'status':'QUALIFIED_COMPARISON_NOT_PROMOTED',
        'report':prep.pin(output),'helper':prep.pin(__file__),
        'payload_sha256':hashlib.sha256(encoded.encode()).hexdigest(),
        'accounts':payload['accounts'],'august_status':payload['august_status'],
        'no_finalists':payload['no_finalists'],'used_to_select':False,
        'evidence':payload['evidence'],'no_process_launched':True}
    prep.write_new(receipt_path,receipt)
    return receipt


TEMPLATE='''<!doctype html><html lang="pl"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>GOD-X7 · pełne porównanie lotowania</title><style>
:root{color-scheme:dark;font:16px system-ui,sans-serif;background:#0d131d;color:#e2eaf5}*{box-sizing:border-box}body{max-width:1450px;margin:32px auto;padding:0 20px}h1{font-size:34px}p{line-height:1.6;color:#bdcadc}section,.card{background:#151f2e;border:1px solid #30405a;border-radius:12px;padding:18px;margin:20px 0}.warning{border-left:4px solid #f3c66c;padding:12px 16px;background:#242432}.cards,.controls{display:flex;gap:12px;flex-wrap:wrap}.card{flex:1;min-width:190px}.card strong{display:block;font-size:24px;margin-top:8px}select,input{font:inherit;color:inherit;background:#243248;border:1px solid #50617b;padding:8px;border-radius:6px;max-width:100%;margin:5px}table{border-collapse:collapse;width:100%;font-size:14px}td,th{padding:9px;border-bottom:1px solid #30405a;text-align:right;white-space:nowrap}td:first-child,th:first-child{text-align:left}th{cursor:pointer;color:#a9c1e3}tr:hover{background:#25354b}tr.control{background:#3a3324}tr.finalist{border-left:3px solid #63dcc0}.scroll{overflow:auto;max-height:550px}.plot-scroll{overflow:auto}svg{width:100%;background:#101926;border-radius:8px}pre{white-space:pre-wrap;overflow-wrap:anywhere;color:#b7cce3}small,.muted{color:#9dafc5}.positive{color:#63dcc0}.negative{color:#ff9494}#tip{position:fixed;display:none;pointer-events:none;max-width:min(340px,calc(100vw - 24px));padding:10px;background:#29405d;border-radius:7px}@media(max-width:700px){#plot{min-width:900px}h1{font-size:28px}}
</style><body><small>GOD-X7 · WOLUMEN KAŻDEJ POZYCJI · PORÓWNANIE DIAGNOSTYCZNE</small>
<h1>Pełne okno: cap 0.01 / 5 / 10</h1>
<p>20 czerwca–1 września 2026 (koniec wyłączny: 2 września) · osobny depozyt 600 USD w każdym przebiegu. Nie sumujemy wyników niezależnych kont.</p>
<p class="warning" id="qualification"></p><p class="muted" id="study"></p>
<p>TRAIN obejmował 20 czerwca–31 lipca, a późniejsza selekcja sierpniowa dwa świeże depozyty 300/600 USD. Poniższe pełne przebiegi nie zmieniają tamtego zamrożonego wyboru. To wcześniej oglądane dane historyczne, nie nietknięty holdout. Eksport zawiera końcową treść wiadomości, nie pełną kolejność edycji live.</p>
<div class="cards" id="cards"></div>
<section><h2>Zysk i drawdown pełnego okna</h2><p>Każdy punkt jest osobnym kontem. Złota obwódka oznacza kontrolę GOD-X7. Kolory oznaczają cap. Kliknięcie otwiera rzeczywiste dni i podsumowanie konfiguracji. Na telefonie wykres i tabele można przewijać poziomo.</p>
<div class="controls"><label>Cap <select id="cap"><option value="all">Wszystkie</option><option value="0.01">0.01</option><option value="5">5</option><option value="10">10</option></select></label><label>Skala zysku <select id="scale"><option value="log">Symetryczna logarytmiczna</option><option value="linear">Liniowa</option></select></label><label>Wariant <input id="query" placeholder="np. 043B lub GOD-X7"></label></div>
<p class="muted">0.01 — niebieski · 5 — fioletowy · 10 — zielony. Maksymalny DD nie jest przycinany do 100%.</p><div class="plot-scroll"><svg id="plot" viewBox="0 0 1100 450" aria-label="Pełny zysk i drawdown"></svg></div></section>
<section><h2>Wszystkie pełne wyniki</h2><p class="muted">Kolumny finansowe opisują pełne okno, nie TRAIN. Kliknij nagłówek, aby sortować. „Źródła” to przyjęte / zaobserwowane niezależne sygnały wejściowe, nie transakcje, fill ani skuteczność.</p>
<div class="scroll"><table><thead><tr><th data-key="id">Konfiguracja</th><th data-key="cap">Cap</th><th data-key="total_profit">Zysk USD</th><th data-key="end_equity">Końcowe equity</th><th data-key="max_dd_abs">DD USD</th><th data-key="max_dd_pct">DD %</th><th data-key="max_daily_rdd_abs">Max dzienny RDD USD</th><th data-key="max_daily_rdd_pct">Max dzienny RDD %</th><th data-key="positive_market_days_pct">Dni+ %</th><th data-key="filled_baskets">Koszyki z pozycjami</th><th data-key="trades">Transakcje</th><th>Źródła</th><th>TRAIN / sierpień</th></tr></thead><tbody id="table"></tbody></table></div></section>
<section id="detail"><h2>Wybierz przebieg</h2><p>Kliknij wiersz lub punkt. Dzienne equity to granice doby; nie przedstawiamy ich jako ścieżki tickowej ani ścieżki salda.</p></section>
<details><summary>Definicje, ograniczenia i dowody</summary><p>DD equity odnosi się do wcześniejszego szczytu; dzienny RDD do początkowego equity dnia. RDD i minima pochodzą wyłącznie z zapisanych obserwacji silnika. Znak — oznacza brak statystyki, nigdy domyślne zero. Maksima DD/RDD w USD i procentach mogą pochodzić z różnych momentów. Brak potwierdzonego próbkowania salda/equity oznacza brak takiego wykresu.</p><p>Krzywe zmieniają lot wraz z kapitałem, podział działa na każdej pozycji, a 10 opcjonalnych osi opisuje wyłącznie przyczynową korektę wolumenu. Dotyczy nadwyżki ponad legalne minimum lota. Włączenie osi nie gwarantuje ograniczenia straty ani zachowania liczby wejść. Ten raport nie promuje GOD-X8 ani nowego presetu do pracy live.</p><pre id="evidence"></pre></details>
<div id="tip"></div><script id="data" type="application/json">__DATA__</script><script>
const D=JSON.parse(document.getElementById('data').textContent),$=id=>document.getElementById(id),fmt=(v,n=2)=>Number.isFinite(v)?v.toLocaleString('pl-PL',{minimumFractionDigits:n,maximumFractionDigits:n}):'—';
const colors={'0.01':'#79b8ff','5':'#b69cff','10':'#63dcc0'};let sort='id',direction=1;
const axes=['equity_stress','portfolio_load','direction_load','basket_count','spread_stress','tp1_deficit','stop_width','age_decay','rearm_decay','day_dd'];
const axisLabels=['equity/saldo','ryzyko portfela','ryzyko kierunku','liczba koszyków','spread / SL','relacja TP1 / SL','SL / szerokość strefy','wiek koszyka','zakończone rearmy','spadek equity dnia'];
$('study').textContent=D.study_name;
$('qualification').textContent=D.no_finalists?'NO_FINALISTS — po sierpniu brak kwalifikujących się finalistów. Pokazujemy diagnostyczne porównanie zamrożonej shortlisty TRAIN; dodatni wynik pełnego okna nie jest promocją.':`Po sierpniu zamrożono ${D.august_finalists.length} finalistów do dalszych badań. Pełne porównanie nie zmienia tej listy i nie certyfikuje GOD-X8.`;
$('cards').innerHTML=[['Osobne konta',D.accounts],['Konfiguracje TRAIN',D.train_shortlist.length],['Finaliści po sierpniu',D.august_finalists.length],['Depozyt każdego konta','600 USD']].map(([a,b])=>`<div class="card">${a}<strong>${b}</strong></div>`).join('');
$('evidence').textContent=JSON.stringify({plan:D.plan,finalFreeze:D.final_freeze,usedForSelection:false},null,2);
function visible(){const cap=$('cap').value,q=$('query').value.toLowerCase();return D.rows.filter(r=>(cap==='all'||String(r.cap)===cap)&&r.id.toLowerCase().includes(q));}
function status(r){return r.control?'Kontrola GOD-X7':r.august_status==='FROZEN_FINALIST'?'Shortlista TRAIN / finalista sierpnia':'Shortlista TRAIN / bez kwalifikacji sierpniowej';}
function sources(r){const s=r.source_statistics;return s?`${s.accepted} / ${s.observed}${s.unattributed?' · nieprzypisane: '+s.unattributed:''}`:'—';}
function show(key){const r=D.rows.find(v=>v.key===key),s=r.sizing;
 $('detail').innerHTML=`<h2>${r.id} · cap ${fmt(r.cap,r.cap===.01?2:0)}</h2><p>${status(r)}. Pełne okno: zysk ${fmt(r.total_profit)} USD, końcowe equity ${fmt(r.end_equity)} USD, końcowe saldo ${fmt(r.end_balance)} USD, DD ${fmt(r.max_dd_pct)}%.</p><p>${r.blown||r.stop_outs>0?'Zaobserwowany stop-out lub ruina. ':''}Krzywa ${s.lot_growth_mode} · kapitał odniesienia ${fmt(s.lot_growth_reference_balance,0)} USD · podział każdej pozycji ${s.lot_growth_allocation}.</p><details><summary>Krzywa, podział i 10 osi</summary><pre id="settings"></pre></details><p class="muted">Wyłącznie dobowe granice i zapisane minima/RDD; brak wykresu tickowego i dziennego salda. Źródła przyjęte / wejściowe: ${sources(r)}.</p><div class="scroll"><table><thead><tr><th>Dzień</th><th>Equity start</th><th>Equity koniec</th><th>Zmiana USD</th><th>Minimum</th><th>DD USD</th><th>RDD USD</th><th>RDD %</th><th>Zamknięcia</th></tr></thead><tbody>${r.days.map(d=>`<tr><td>${d.date}</td>${['start_equity','end_equity','profit','min_equity','max_dd','real_dd','real_dd_pct'].map(k=>`<td>${fmt(d[k])}</td>`).join('')}<td>${fmt(d.trades,0)}</td></tr>`).join('')}</tbody></table></div>`;
 $('settings').textContent=JSON.stringify({krzywa:s.lot_growth_mode,lotOdniesienia:s.lot_growth_reference_lot,kapitalOdniesienia:s.lot_growth_reference_balance,power:s.lot_growth_power,rate:s.lot_growth_rate_pct,capitalMultiple:s.lot_growth_capital_multiple,lotMultiple:s.lot_growth_lot_multiple,podzial:s.lot_growth_allocation,budzetDodatkowyPct:s.lot_growth_basket_risk_pct,osie:Object.fromEntries(axes.map((a,i)=>[axisLabels[i],s['lot_growth_'+a+'_strength']]))},null,2);}
function table(){const rows=visible();rows.sort((a,b)=>{const x=a[sort],y=b[sort];if(x==null||y==null)return x==null?(y==null?0:1):-1;return direction*(typeof x==='string'?x.localeCompare(y):x-y)});
 $('table').innerHTML=rows.map(r=>`<tr data-key="${r.key}" class="${r.control?'control':r.august_status==='FROZEN_FINALIST'?'finalist':''}"><td>${r.id}</td><td>${fmt(r.cap,r.cap===.01?2:0)}</td>${['total_profit','end_equity','max_dd_abs','max_dd_pct','max_daily_rdd_abs','max_daily_rdd_pct','positive_market_days_pct','filled_baskets','trades'].map(k=>`<td>${fmt(r[k],['filled_baskets','trades'].includes(k)?0:2)}</td>`).join('')}<td>${sources(r)}</td><td>${status(r)}</td></tr>`).join('');$('table').querySelectorAll('tr').forEach(r=>r.onclick=()=>show(r.dataset.key));}
function plot(){const points=visible(),s=$('plot'),log=$('scale').value==='log',tx=v=>log?Math.sign(v)*Math.log10(1+Math.abs(v)):v,xmax=Math.ceil(Math.max(100,...points.map(p=>p.max_dd_pct))/10)*10,lo=Math.min(0,...points.map(p=>tx(p.total_profit))),hi=Math.max(1,...points.map(p=>tx(p.total_profit))),x=v=>75+v/xmax*980,y=v=>385-(tx(v)-lo)/(hi-lo)*345;let svg='';for(let n=0;n<=10;n++){const v=xmax*n/10;svg+=`<path d="M${x(v)} 25V385" stroke="#30405a"/><text x="${x(v)}" y="410" fill="#afc1d8" text-anchor="middle">${fmt(v,0)}%</text>`;}for(let i=0;i<=4;i++){const z=lo+(hi-lo)*i/4,raw=log?Math.sign(z)*(10**Math.abs(z)-1):z;svg+=`<path d="M75 ${y(raw)}H1055" stroke="#30405a"/><text x="70" y="${y(raw)-4}" text-anchor="end" fill="#afc1d8" font-size="12">${fmt(raw,0)}</text>`;}svg+='<text x="550" y="441" fill="#cbd7e7" text-anchor="middle">Maksymalny DD equity pełnego okna</text>';for(const r of points)svg+=`<circle data-key="${r.key}" cx="${x(r.max_dd_pct)}" cy="${y(r.total_profit)}" r="${r.control?6:4}" fill="${colors[String(r.cap)]}" stroke="${r.control?'#f3c66c':'none'}" stroke-width="2"/>`;s.innerHTML=svg;s.querySelectorAll('circle').forEach(c=>{const r=points.find(v=>v.key===c.dataset.key);c.onclick=()=>show(r.key);c.onpointermove=e=>{$('tip').textContent=`${r.id} · cap ${r.cap} · zysk ${fmt(r.total_profit)} USD · DD ${fmt(r.max_dd_pct)}%`;$('tip').style.cssText=`display:block;left:${Math.max(8,Math.min(innerWidth-350,e.clientX+10))}px;top:${Math.max(8,Math.min(innerHeight-90,e.clientY+10))}px`};c.onpointerleave=()=>$('tip').style.display='none'});}
function update(){table();plot()}$('cap').onchange=update;$('query').oninput=update;$('scale').onchange=plot;document.querySelectorAll('th[data-key]').forEach(h=>h.onclick=()=>{direction=sort===h.dataset.key?-direction:1;sort=h.dataset.key;table()});update();
</script></body></html>'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('study',type=Path);parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args();print(json.dumps(render(args.study,args.output)['report']))


if __name__=='__main__':main()
