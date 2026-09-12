"""Render a private standalone audit of the completed 300-preset TRAIN study."""
from __future__ import annotations
import argparse, copy, hashlib, json, statistics
from collections import Counter
from pathlib import Path
from prepare_lot_growth_sweep import read, pin, fingerprint, verify_pin, write_new
from rank_lot_growth_sweep import verified_results
from validate_lot_growth_finalists import train_contract, unique_pins

def build_payload(study):
    root=Path(study).resolve()
    # Recompute the complete TRAIN ranking from qualified receipts. A pinned
    # preregistration alone does not establish that a saved selection is true.
    verified=train_contract(root)
    evidence=unique_pins(list(verified['inputs'].values())+verified['evidence']+
        [pin(__file__),pin(Path(__file__).with_name('validate_lot_growth_finalists.py'))])
    selection=read(root/'TRAIN_SELECTION.json');prereg=verified['prereg']
    summaries,details=verified_results(root,'sizing300')
    controls,control_details=verified_results(root,'controls')
    rows=copy.deepcopy(selection['all_candidates']);registry={r['id']:r for r in prereg['candidates']}
    if set(summaries)!=set(registry) or len(rows)!=300 or {r['id'] for r in rows}!=set(registry):
        raise ValueError('Incomplete study')
    byid={r['id']:r for r in rows};pairs=[]
    for n in range(1,151):
        x=byid[f'G8-sizing-{n:03d}A'];y=byid[f'G8-sizing-{n:03d}B']
        pairs.append({'pair':n,'dd_change':y['max_dd_pct']-x['max_dd_pct'],
                      'profit_change':y['total_profit']-x['total_profit'],
                      'green_change':y['positive_market_days_pct']-x['positive_market_days_pct']})
    duplicates=Counter()
    for r in rows:
        # verified_results also validates an existing empty ledger; zero trades
        # may legitimately have no ledger file, and is not missing execution.
        trade=details[r['id']]['transakcje']
        closed=[] if trade is None else trade['transakcje']
        # This fingerprint means closed-execution equality, not identical full state.
        digest=fingerprint(closed)
        r['closed_execution_sha256']=digest;duplicates[digest]+=1
        r['curve']=registry[r['id']]['curve'];r['allocation']=registry[r['id']]['allocation']
        r['anchor']=registry[r['id']]['anchor'];r['arm']=registry[r['id']]['arm']
        r['axes']=registry[r['id']]['active_stress_axes'];r['settings']=registry[r['id']]['changes']
        r['days']=details[r['id']]['dni']
        r['lot_sizing_diagnostics']=summaries[r['id']].get('lot_sizing_diagnostics',{})
    payload={'schema':'conduit.sizing300.train-report.v1','scope':'TRAIN_ONLY',
        'study_name':verified['plan']['name'],'rows':rows,'selected':[r['id'] for r in selection['selected']],
        'eligible':selection['eligible_count'],'controls':controls,'pairs':pairs,
        'duplicate_groups':sum(n>1 for n in duplicates.values()),
        'median_dd_change':statistics.median(p['dd_change'] for p in pairs),
        'median_profit_change':statistics.median(p['profit_change'] for p in pairs),
        'dd_improved_pairs':sum(p['dd_change']<0 for p in pairs),
        'plan_sha256':pin(root/'PLAN.json')['sha256'],'selection_sha256':pin(root/'TRAIN_SELECTION.json')['sha256'],
        'certified_god_x8':False,'evidence':evidence}
    # Preserve null statistics (e.g. log growth after insolvency), but never
    # emit NaN/Infinity or convert an invalid value into a financial zero.
    json.dumps(payload,allow_nan=False)
    for record in evidence:verify_pin(record)
    return payload

def render(study, output_path):
    output_path=Path(output_path).resolve();receipt_path=output_path.with_suffix(output_path.suffix+'.receipt.json')
    if output_path.exists() or receipt_path.exists():raise FileExistsError(output_path)
    payload=build_payload(study)
    encoded=json.dumps(payload,ensure_ascii=False,allow_nan=False).replace('<','\\u003c')
    output=TEMPLATE.replace('__DATA__',encoded)
    output_path.parent.mkdir(parents=True,exist_ok=True)
    with output_path.open('x',encoding='utf-8') as f:f.write(output)
    receipt={'schema':'conduit.sizing300.train-report-receipt.v1','status':'QUALIFIED_TRAIN_REPORT',
        'report':pin(output_path),'helper':pin(__file__),'payload_sha256':hashlib.sha256(encoded.encode()).hexdigest(),
        'rows':300,'pairs':150,'controls':len(payload['controls']),'scope':'TRAIN_ONLY',
        'certified_god_x8':False,'selection_recomputed_from_completed_inputs':True,'evidence':payload['evidence']}
    write_new(receipt_path,receipt)
    return receipt

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('study',type=Path);ap.add_argument('--output',type=Path,required=True)
    a=ap.parse_args()
    print(json.dumps(render(a.study,a.output)['report']))

TEMPLATE='''<!doctype html><html lang="pl"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>GOD-X7 · badanie lotowania 300 konfiguracji</title><style>
:root{color-scheme:dark;font-family:system-ui,sans-serif;background:#0d131d;color:#e1e9f3}body{max-width:1400px;margin:40px auto;padding:0 24px}h1{font-size:36px;margin-bottom:8px}p{line-height:1.6;color:#bcc9d8}.muted,small{color:#9dafc5}.cards{display:flex;gap:16px;flex-wrap:wrap;margin:24px 0}.card,section{background:#151f2e;border:1px solid #30405a;border-radius:14px;padding:20px}.card{flex:1;min-width:200px}.card strong{font-size:25px;display:block;margin-top:8px}section{margin:22px 0}button,select,input{font:inherit;background:#243248;color:#edf3ff;border:1px solid #50617b;padding:9px;border-radius:7px;margin:4px}table{border-collapse:collapse;width:100%;font-size:14px}td,th{padding:10px;text-align:right;border-bottom:1px solid #30405a;white-space:nowrap}td:first-child,th:first-child{text-align:left}th{cursor:pointer;color:#a9c1e3;position:sticky;top:0;background:#151f2e}tr:hover{background:#25354b}tr.selected{background:#213e45}.scroll{overflow:auto;max-height:560px}.green{color:#63dcc0}.red{color:#ff9494}svg{width:100%;height:auto;background:#101926;border-radius:12px}details{margin:20px 0}pre{white-space:pre-wrap;word-break:break-word;color:#b7cce3}a{color:#8ecaff}#tip{position:fixed;pointer-events:none;background:#263b54;padding:10px;border-radius:8px;display:none;max-width:min(350px,calc(100vw - 32px));box-sizing:border-box}.warning{border-left:4px solid #f3c66c;padding-left:15px}.controls{display:flex;gap:12px;flex-wrap:wrap;align-items:center}.plot-scroll{overflow-x:auto}@media(max-width:700px){#plot{min-width:900px}}
</style><body><small>GIGA_SWEEP8 · SYNERGY · TYLKO WOLUMEN POZYCJI</small>
<h1>300 konfiguracji lotowania GOD-X7</h1><p>TRAIN: 20 czerwca–31 lipca 2026 · 600 USD · max lot 5 · dokładne ticki · 150 par A/B</p>
<p class="warning">To wynik etapu selekcji. GOD-X7 pozostaje głównym presetem. Warianty nie zostały tutaj zatwierdzone jako GOD-X8. Sierpień, wrzesień oraz przebiegi od świeżych depozytów wymagają osobnych wyników. Historyczne dane zawierają końcowe wersje wiadomości, których wcześniejsze edycje nie zawsze są dostępne.</p>
<p class="muted" id="study-name"></p><div class="cards" id="cards"></div>
<section><h2>Zysk i drawdown wszystkich wariantów</h2><p>Każdy punkt to pełny przebieg TRAIN. Poziomo: maksymalny DD equity; pionowo: zysk netto. Najedź na punkt, aby zobaczyć wynik. Kliknięcie wariantu A/B otwiera dni i ustawienia. Na wąskim ekranie wykres i tabele można przewijać poziomo.</p>
<div class="controls"><label>Skala zysku <select id="scale"><option value="log">Symetryczna logarytmiczna</option><option value="linear">Liniowa</option></select></label><span class="green">● A: sama krzywa i podział lota</span><span style="color:#b69cff">● B: dodatkowe osie</span><span style="color:#f3c66c">● GOD-X7</span></div><div class="plot-scroll"><svg id="plot" viewBox="0 0 1100 450" aria-label="Zysk względem drawdown"></svg></div></section>
<section><h2>Porównanie 150 par</h2><p id="paired"></p><p class="muted">Różnica B−A opisuje cały przebieg, wraz z wpływem wolumenu na dalsze decyzje i dostępność margin. Równe zamknięte transakcje nie dowodzą identyczności całego stanu koszyków. Nie zastępujemy duplikatów nowymi próbami.</p></section>
<section><h2>Wszystkie wyniki</h2><div class="controls"><label>Szukaj <input id="query" placeholder="np. 043B"></label><label>Widok <select id="filter"><option value="all">300 wariantów</option><option value="eligible">Spełnia bramki TRAIN</option><option value="selected">Wybrane do walidacji</option></select></label><span>Kliknij nagłówek, aby sortować.</span></div><div class="scroll"><table><thead><tr><th data-key="id">Wariant</th><th data-key="total_profit">Zysk USD</th><th data-key="max_dd_pct">DD %</th><th data-key="positive_market_days_pct">Dni+ %</th><th data-key="filled_baskets">Koszyki z pozycjami</th><th data-key="active_entry_days">Dni wejść</th><th data-key="trades">Transakcje</th><th data-key="max_daily_rdd_pct">Najgorszy RDD %</th><th>Bramki</th></tr></thead><tbody id="table"></tbody></table></div></section>
<section id="detail"><h2>Wybierz wariant</h2><p>Kliknij wiersz lub punkt na wykresie.</p></section>
<details><summary>Reguły i identyfikacja badania</summary><p>Nowe osie: equity/balance, obciążenie portfela i kierunku, liczba koszyków, spread, relacja TP1/SL, szerokość stopa, wiek koszyka, liczba rearmów oraz spadek equity od początku dnia. Łączenie: minimum mnożników, zastosowanie do części lota ponad legalne minimum. SL, TP, kierunek i dotychczasowe ustawienia strategii pozostają w konfiguracjach GOD-X7.</p><p>RDD dnia = max(0, equity początkowe − minimalne equity dnia). Procent RDD odnosi się do początkowego equity dnia. DD odnosi się do wcześniejszego szczytu equity. Dni+ obejmują wszystkie dni rynkowe w oknie, także dni bez transakcji. Znak — oznacza niedostępną statystykę, nie zero; RDD nie jest przybliżane z zapisanej krzywej.</p><p>Bramki TRAIN: dodatni wynik, DD≤35%, dni+≥50%, brak stop-out i ruiny, co najmniej połowa koszyków z pozycjami oraz 80% dni wejść kontroli stałego 0.01. Są to warunki dopuszczenia do dalszych testów, nie osiągnięcie celu 90–100% dni dodatnich.</p><pre id="hashes"></pre></details>
<div id="tip"></div><script id="data" type="application/json">__DATA__</script><script>
const D=JSON.parse(document.getElementById('data').textContent),$=id=>document.getElementById(id),fmt=(v,n=2)=>Number.isFinite(v)?v.toLocaleString('pl-PL',{maximumFractionDigits:n,minimumFractionDigits:n}):'—';
let sort='max_dd_pct',direction=1;
$('study-name').textContent=D.study_name;
$('cards').innerHTML=[['Przebiegi','300 / 300'],['Spełnia bramki TRAIN',D.eligible],['Wybrane do walidacji',D.selected.length],['Domyślny preset','GOD-X7']].map(([k,v])=>`<div class="card">${k}<strong>${v}</strong></div>`).join('');
$('paired').textContent=`B zmniejszyło DD w ${D.dd_improved_pairs} ze 150 par. Mediana zmiany DD: ${fmt(D.median_dd_change)} pp; mediana zmiany zysku: ${fmt(D.median_profit_change)} USD. Grup z identycznymi zamkniętymi transakcjami: ${D.duplicate_groups}.`;
$('hashes').textContent=`SHA-256 planu: ${D.plan_sha256}\nSHA-256 selekcji: ${D.selection_sha256}`;
function show(id){let r=D.rows.find(x=>x.id===id);$('detail').innerHTML=`<h2>${r.id}</h2><p>${r.curve} · próg ${fmt(r.anchor,0)} USD · ${r.allocation} · wariant ${r.arm}</p><p>${r.blown||r.stop_outs>0?'Zaobserwowany stop-out lub ruina. ':''}${r.rejections.length?'Nie spełnia: '+r.rejections.join(', '):'Spełnia bramki TRAIN; dalsza walidacja wymagana.'}</p><details><summary>Ustawienia zmienione względem GOD-X7</summary><pre id="settings"></pre></details><div class="scroll"><table><thead><tr><th>Dzień</th><th>Equity start USD</th><th>Equity koniec USD</th><th>Zmiana USD</th><th>Minimum USD</th><th>RDD USD</th><th>RDD %</th><th>Zamknięcia</th></tr></thead><tbody>${r.days.map(d=>`<tr><td>${d.date}</td><td>${fmt(d.start_equity)}</td><td>${fmt(d.end_equity)}</td><td class="${Number.isFinite(d.profit)?(d.profit>=0?'green':'red'):''}">${fmt(d.profit)}</td><td>${fmt(d.min_equity)}</td><td>${fmt(d.real_dd)}</td><td>${fmt(d.real_dd_pct)}</td><td>${fmt(d.trades,0)}</td></tr>`).join('')}</tbody></table></div>`;$('settings').textContent=JSON.stringify(r.settings,null,2);}
function table(){let q=$('query').value.toLowerCase(),f=$('filter').value;let rows=D.rows.filter(r=>r.id.toLowerCase().includes(q)&&(f==='all'||(f==='eligible'?!r.rejections.length:D.selected.includes(r.id))));rows.sort((a,b)=>direction*(typeof a[sort]==='string'?a[sort].localeCompare(b[sort]):(a[sort]??-Infinity)-(b[sort]??-Infinity)));$('table').innerHTML=rows.map(r=>`<tr data-id="${r.id}" class="${D.selected.includes(r.id)?'selected':''}"><td>${r.id}</td>${['total_profit','max_dd_pct','positive_market_days_pct','filled_baskets','active_entry_days','trades','max_daily_rdd_pct'].map(k=>`<td>${fmt(r[k],['filled_baskets','active_entry_days','trades'].includes(k)?0:2)}</td>`).join('')}<td>${r.rejections.length?'Nie':'Tak'}</td></tr>`).join('');$('table').querySelectorAll('tr').forEach(tr=>tr.onclick=()=>show(tr.dataset.id));}
function plot(){const s=$('plot'),log=$('scale').value==='log',tx=v=>log?Math.sign(v)*Math.log10(1+Math.abs(v)):v;const points=[...D.rows,...Object.entries(D.controls).map(([id,m])=>({...m,id,arm:'control'}))],lo=Math.min(0,...points.map(p=>tx(p.total_profit))),hi=Math.max(1,...points.map(p=>tx(p.total_profit))),xmax=Math.ceil(Math.max(100,...points.map(p=>p.max_dd_pct))/10)*10,x=v=>70+v/xmax*1000,y=v=>390-(tx(v)-lo)/(hi-lo)*350;let svg='';for(let n=0;n<=xmax;n+=xmax/10)svg+=`<path d="M${x(n)} 25V390" stroke="#2d3b50"/><text x="${x(n)}" y="416" fill="#adbdd1" text-anchor="middle">${n}%</text>`;for(let n=0;n<=4;n++){let v=lo+(hi-lo)*n/4,raw=log?Math.sign(v)*(10**Math.abs(v)-1):v;svg+=`<path d="M70 ${y(raw)}H1070" stroke="#2d3b50"/><text x="64" y="${y(raw)-5}" fill="#adbdd1" text-anchor="end" font-size="12">${fmt(raw,0)}</text>`;}svg+='<text x="550" y="441" fill="#cbd7e7" text-anchor="middle">Maksymalny DD equity</text>';for(let p of points)svg+=`<circle data-id="${p.id}" cx="${x(p.max_dd_pct)}" cy="${y(p.total_profit)}" r="${p.arm==='control'?6:4}" fill="${p.arm==='control'?'#f3c66c':p.arm==='A'?'#63dcc0':'#b69cff'}" opacity=".8"/>`;s.innerHTML=svg;s.querySelectorAll('circle').forEach(c=>{let p=points.find(v=>v.id===c.dataset.id);c.onpointermove=e=>{$('tip').textContent=`${p.id} · DD ${fmt(p.max_dd_pct)}% · zysk ${fmt(p.total_profit)} USD · dni+ ${fmt(p.positive_market_days_pct)}%`;$('tip').style.cssText=`display:block;left:${Math.max(8,Math.min(innerWidth-360,e.clientX+12))}px;top:${Math.max(8,Math.min(innerHeight-100,e.clientY+12))}px`};c.onpointerleave=()=>$('tip').style.display='none';c.onclick=()=>{if(p.arm!=='control')show(p.id)}});}
$('query').oninput=table;$('filter').onchange=table;$('scale').onchange=plot;document.querySelectorAll('th[data-key]').forEach(h=>h.onclick=()=>{direction=sort===h.dataset.key?-direction:1;sort=h.dataset.key;table()});table();plot();
</script></body></html>'''

if __name__=='__main__':main()
