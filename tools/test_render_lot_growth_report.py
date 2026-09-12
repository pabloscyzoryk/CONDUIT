"""Synthetic, offline report evidence. Never runs BTP or reads actual candidates."""
import copy
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

import prepare_lot_growth_sweep as prep
import rank_lot_growth_sweep as rank
import render_lot_growth_report as report
from test_validate_lot_growth_finalists import fixture


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, allow_nan=False), encoding='utf-8')


def make_report_study(root):
    root=fixture(root)
    folder=root/'sizing300/results';summary=prep.read(folder/'wyniki_compound.json')
    receipt=prep.read(root/'sizing300/receipt.json')
    for name,zero in [('G8-sizing-001A',False),('G8-sizing-001B',True)]:
        data_path=folder/(name+'_compound_dane.json');data=prep.read(data_path);m=summary[name]
        if zero:
            m.update(trades=0,baskets=0,total_profit=0,end_equity=600,min_equity=600,
                     positive_market_days_pct=0,max_dd_pct=0)
            basket_path=folder/(name+'_compound_koszyki.json');save(basket_path,[])
            ledger_path=folder/(name+'_compound_transakcje.json')
            ledger_path.unlink() # Exact synthetic file; BTP may omit a zero-trade ledger.
            receipt['detail_artifacts'].pop(ledger_path.name)
        else:
            m.update(total_profit=-700,end_equity=-100,min_equity=-100,max_dd_pct=125,
                     positive_market_days_pct=0,blown=True,stop_outs=1)
        for index,day in enumerate(data['dni']):
            start=600 if zero else (600 if index==0 else -50)
            end=600 if zero else (-50 if index==0 else -100)
            day.update(start_equity=start,end_equity=end,profit=end-start,
                       min_equity=min(start,end),real_dd=None,real_dd_pct=None,
                       trades=0 if zero else 1)
        data['metryki']=m;save(data_path,data)
    save(folder/'wyniki_compound.json',summary)
    receipt['result_sha256']={'wyniki_compound.json':prep.pin(folder/'wyniki_compound.json')['sha256']}
    for name in receipt['detail_artifacts']:
        p=prep.pin(folder/name);receipt['detail_artifacts'][name]={'bytes':p['bytes'],'sha256':p['sha256']}
    save(root/'sizing300/receipt.json',receipt)
    summary,details=rank.verified_results(root,'sizing300')
    controls,control_details=rank.verified_results(root,'controls')
    registry={r['id']:r for r in prep.read(root/'PREREGISTRATION.json')['candidates']}
    name='GOD-X7-fixed001';d=control_details[name]
    baseline=rank.describe(name,controls[name],d,d['koszyki'],'reference',trades=d['transakcje'])
    rows=[rank.describe(name,m,details[name],details[name]['koszyki'],registry[name]['settings_sha256'],
                        trades=details[name]['transakcje']) for name,m in summary.items()]
    selection=rank.select(rows,baseline)
    selection.update(schema='conduit.sizing300.train-selection.v1',baseline=baseline,
        preregistration=prep.pin(root/'PREREGISTRATION.json'),plan=prep.pin(root/'PLAN.json'),
        completion_receipts=[prep.pin(root/job/'receipt.json') for job in ('controls','sizing300')])
    save(root/'TRAIN_SELECTION.json',selection)
    save(root/'SHORTLIST.json',[registry[r['id']] for r in selection['selected']])
    return root


class ReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp=tempfile.TemporaryDirectory()
        cls.root=Path(cls.temp.name)
        cls.study=make_report_study(cls.root/'synthetic_train')

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def edit(self,path,modify,allow_nan=False):
        original=path.read_bytes();self.addCleanup(path.write_bytes,original)
        doc=prep.read(path);modify(doc)
        path.write_text(json.dumps(doc,allow_nan=allow_nan),encoding='utf-8')

    def test_actual_generator_preserves_all_300_rows_controls_and_pairs(self):
        output=self.root/'SYNTHETIC_REPORT.html';receipt=report.render(self.study,output)
        text=output.read_text('utf-8')
        encoded=re.search(r'<script id="data" type="application/json">(.*?)</script>',text,re.S).group(1)
        payload=json.loads(encoded)
        self.assertEqual(hashlib.sha256(encoded.encode()).hexdigest(),receipt['payload_sha256'])
        self.assertEqual(receipt['report'],prep.pin(output))
        self.assertEqual(len(payload['rows']),300);self.assertEqual(len(payload['pairs']),150)
        self.assertEqual(payload['controls'],prep.read(self.study/'controls/results/wyniki_compound.json'))
        metrics=prep.read(self.study/'sizing300/results/wyniki_compound.json')
        for row in payload['rows']:
            for key in ('total_profit','max_dd_pct','trades','baskets','positive_market_days_pct'):
                self.assertEqual(row[key],metrics[row['id']][key])
        self.assertEqual(payload['scope'],'TRAIN_ONLY');self.assertFalse(payload['certified_god_x8'])
        self.assertIn('Synthetic only',text)
        self.assertIn('nie zostały tutaj zatwierdzone jako GOD-X8',text)
        node=shutil.which('node') or ('C:/nvm4w/nodejs/node.exe' if Path('C:/nvm4w/nodejs/node.exe').exists() else None)
        self.assertIsNotNone(node,'Node required for the standalone JavaScript syntax check')
        javascript=text.rsplit('<script>',1)[1].split('</script>',1)[0]
        script=self.root/'report_script.js';script.write_text(javascript,encoding='utf-8')
        parsed=subprocess.run([node,'--check',str(script)],capture_output=True,text=True,check=False,timeout=30)
        self.assertEqual(parsed.returncode,0,parsed.stderr)

    def test_selection_metric_and_duplicate_identity_tamper_are_rejected(self):
        path=self.study/'TRAIN_SELECTION.json'
        original=path.read_bytes()
        for mutate in (lambda d:d['all_candidates'][0].update(total_profit=999999),
                       lambda d:d['all_candidates'].__setitem__(0,copy.deepcopy(d['all_candidates'][1]))):
            with self.subTest(mutation=str(mutate)):
                self.edit(path,mutate)
                with self.assertRaisesRegex(ValueError,'not recomputed'):report.build_payload(self.study)
                path.write_bytes(original)

    def test_selection_plan_pin_and_selected_list_are_recomputed(self):
        path=self.study/'TRAIN_SELECTION.json';original=path.read_bytes()
        for mutate in (lambda d:d['plan'].update(sha256='0'*64),lambda d:d['selected'].reverse()):
            with self.subTest(mutation=str(mutate)):
                self.edit(path,mutate)
                with self.assertRaises(ValueError):report.build_payload(self.study)
                path.write_bytes(original)

    def test_unbound_daily_change_is_rejected(self):
        path=self.study/'sizing300/results/G8-sizing-002A_compound_dane.json'
        self.edit(path,lambda d:d['dni'][0].update(real_dd_pct=999))
        with self.assertRaisesRegex(ValueError,'changed after completion'):report.build_payload(self.study)

    def test_nonfinite_saved_selection_is_rejected_without_html(self):
        path=self.study/'TRAIN_SELECTION.json'
        self.edit(path,lambda d:d['all_candidates'][0].update(log_growth=float('-inf')),allow_nan=True)
        output=self.root/'must_not_exist.html'
        with self.assertRaises(ValueError):report.render(self.study,output)
        self.assertFalse(output.exists())

    def test_valid_insolvency_and_zero_trade_ledger_remain_observable(self):
        payload=report.build_payload(self.study);rows={r['id']:r for r in payload['rows']}
        failed=rows['G8-sizing-001A'];zero=rows['G8-sizing-001B']
        self.assertEqual(failed['max_dd_pct'],125);self.assertIsNone(failed['log_growth'])
        # The pinned selector rejects undefined log growth before its later
        # insolvency reasons; the report must retain that exact classification.
        self.assertEqual(failed['rejections'],['invalid_or_incomplete_metrics'])
        self.assertTrue(failed['blown']);self.assertEqual(failed['stop_outs'],1)
        self.assertEqual(zero['closed_execution_sha256'],prep.fingerprint([]))
        self.assertEqual(zero['active_entry_days'],0);self.assertIsNone(zero['max_daily_rdd_pct'])
        self.assertFalse((self.study/'sizing300/results/G8-sizing-001B_compound_transakcje.json').exists())
        self.assertNotIn('Math.min(100,v)',report.TEMPLATE)

    def test_preexisting_output_refuses_before_input_reads(self):
        output=self.root/'existing.html';output.write_text('retain',encoding='utf-8')
        with self.assertRaises(FileExistsError):report.render(self.root/'absent',output)
        self.assertEqual(output.read_text('utf-8'),'retain')


if __name__=='__main__':unittest.main()
