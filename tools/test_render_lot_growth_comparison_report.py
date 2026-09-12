"""Small offline synthetic-report tests. No BTP/MT5 or actual study data."""
import copy
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

import prepare_lot_growth_sweep as prep
import prepare_lot_growth_comparison as comparison
import validate_lot_growth_finalists as august
import test_validate_lot_growth_finalists as fixtures
import render_lot_growth_comparison_report as report


def make_comparison_fixture(root):
    root=Path(root);root.mkdir()
    train=fixtures.fixture(root/'train');validation=root/'august';august.prepare(train,validation)
    plan=prep.read(validation/'PLAN.json');original=fixtures.describe_fixture
    def august_rejections(name,balance,dates,sha,**kwargs):
        return original(name,balance,dates,sha,dd=20 if name in august.CONTROL_IDS else 40)
    with mock.patch.object(fixtures,'describe_fixture',side_effect=august_rejections):
        for job in plan['jobs']:
            fixtures.completed(validation/'PLAN.json',job['id'],plan['finalists_contract']['registry'],
                               ['2026-08-03','2026-08-04'])
    final=root/'final';august.select(validation,final)
    full=root/'full';comparison.prepare(final,full,4);plan=prep.read(full/'PLAN.json')
    for job in plan['jobs']:
        cap=next(iter(plan['comparison_contract']['registry'][job['id']].values()))['cap']
        def full_result(name,balance,dates,sha,**kwargs):
            profit=-50 if cap==10 else cap*20
            m,d,b,t,r=original(name,balance,dates,sha,profit=profit)
            m.update(end_balance=balance+profit,max_dd_abs=30)
            if cap==5:
                m['stat_sygnalow']={'lejek':{'source_observation_version':1,'sygnaly_wejsciowe':10,
                    'koszyki_sygnaly':3,'unattributed_entry_outcomes':0,'accepted_entry_sources_pct':30,
                    'source_identity_semantics':'SYNTHETIC source IDs'}}
            for day in d['dni']:
                day.update(profit=day['end_equity']-day['start_equity'],min_equity=day['start_equity']-3,
                           max_dd=4,real_dd=3,real_dd_pct=3/day['start_equity']*100,trades=1)
            return m,d,b,t,r
        with mock.patch.object(fixtures,'describe_fixture',side_effect=full_result):
            fixtures.completed(full/'PLAN.json',job['id'],plan['comparison_contract']['registry'][job['id']],
                               ['2026-06-22','2026-09-01'])
    return full


class FullReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp=tempfile.TemporaryDirectory();cls.root=Path(cls.temp.name)
        cls.full=make_comparison_fixture(cls.root/'SYNTHETIC_ONLY')

    @classmethod
    def tearDownClass(cls):cls.temp.cleanup()

    def edit(self,path,modify):
        original=path.read_bytes();self.addCleanup(path.write_bytes,original)
        value=prep.read(path);modify(value);path.write_text(json.dumps(value),encoding='utf-8')

    def test_complete_report_matches_all_33_accounts_and_no_finalists(self):
        out=self.root/'SYNTHETIC_FULL_REPORT.html';receipt=report.render(self.full,out)
        document=out.read_text('utf-8')
        encoded=re.search(r'<script id="data" type="application/json">(.*?)</script>',document,re.S).group(1)
        payload=json.loads(encoded)
        self.assertEqual(receipt['report'],prep.pin(out));self.assertEqual(payload['accounts'],33)
        self.assertTrue(payload['no_finalists']);self.assertEqual(payload['august_finalists'],[])
        self.assertEqual(payload['august_status'],'NO_QUALIFYING_FINALIST');self.assertFalse(payload['used_to_select'])
        self.assertEqual({r['cap'] for r in payload['rows']},{.01,5,10})
        self.assertEqual(sum(r['control'] for r in payload['rows']),3)
        for row in payload['rows']:
            metrics=prep.read(self.full/row['job']/'results/wyniki_compound.json')[row['id']]
            for key in ('total_profit','end_equity','end_balance','max_dd_abs','max_dd_pct','trades','baskets'):
                self.assertEqual(row[key],metrics[key])
            self.assertEqual(row['deposit'],600);self.assertIsNone(row['daily_balance_path'])
            self.assertEqual(row['equity_path'],'daily_boundaries_only')
            if row['cap']==5:
                self.assertEqual(row['source_statistics']['accepted'],3)
                self.assertEqual(row['source_statistics']['observed'],10)
                self.assertNotEqual(row['source_statistics']['accepted'],row['filled_baskets'])
            else:self.assertIsNone(row['source_statistics'])
        node=shutil.which('node') or 'C:/nvm4w/nodejs/node.exe'
        script=self.root/'report.js';script.write_text(document.rsplit('<script>',1)[1].split('</script>',1)[0],encoding='utf-8')
        result=subprocess.run([node,'--check',str(script)],capture_output=True,text=True,timeout=30)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('NO_FINALISTS',document);self.assertNotIn('<canvas',document)

    def test_incomplete_third_cap_cannot_render_partial_report(self):
        path=self.full/'owner_cap10/receipt.json';self.edit(path,lambda r:r.update(status='running'))
        out=self.root/'incomplete.html'
        with self.assertRaisesRegex(ValueError,'complete'):report.render(self.full,out)
        self.assertFalse(out.exists())

    def test_bound_daily_artifact_change_is_rejected(self):
        path=next((self.full/'owner_cap001/results').glob('*_dane.json'))
        self.edit(path,lambda d:d['dni'][0].update(real_dd=999))
        with self.assertRaises(ValueError):report.build_payload(self.full)

    def test_existing_output_refuses_before_evidence_reads(self):
        out=self.root/'existing.html';out.write_text('preserve',encoding='utf-8')
        with self.assertRaises(FileExistsError):report.render(self.root/'absent',out)
        self.assertEqual(out.read_text('utf-8'),'preserve')

    def test_source_fields_are_not_inferred_from_basket_counts_or_legacy_zeros(self):
        self.assertIsNone(report.source_statistics({'baskets':100,'stat_sygnalow':{'lejek':{'koszyki_sygnaly':99}}}))
        record={'source_observation_version':1,'sygnaly_wejsciowe':0,'koszyki_sygnaly':0,
                'unattributed_entry_outcomes':0,'accepted_entry_sources_pct':0,'source_identity_semantics':'fixture'}
        result=report.source_statistics({'stat_sygnalow':{'lejek':record}})
        self.assertEqual(result['observed'],0);self.assertEqual(result['accepted'],0)
        bad=copy.deepcopy(record);bad['koszyki_sygnaly']=False
        self.assertIsNone(report.source_statistics({'stat_sygnalow':{'lejek':bad}}))
        bad=copy.deepcopy(record);bad['accepted_entry_sources_pct']=float('inf')
        with self.assertRaises(ValueError):report.source_statistics({'stat_sygnalow':{'lejek':bad}})

    def test_optional_financial_observations_remain_null_and_reject_invalid(self):
        self.assertIsNone(report.optional_number(None));self.assertEqual(report.optional_number(-100),-100)
        for bad in (False,'0',float('inf'),float('-inf'),float('nan')):
            with self.assertRaises(ValueError):report.optional_number(bad)


if __name__=='__main__':unittest.main()
