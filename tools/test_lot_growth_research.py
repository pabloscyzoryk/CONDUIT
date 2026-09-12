"""Guard against changing GOD-X7 strategy or selecting sparse/failed runs."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from prepare_lot_growth_sweep import (candidates, NEW_FIELDS, STRESS_FIELDS, pin,
                                      write_new, verify_build, recipe, fingerprint)
from prepare_lot_growth_sweep import settings_equal
import prepare_lot_growth_sweep as preparer
from collections import Counter
from rank_lot_growth_sweep import (select, describe, verified_results, weekly_median,
                                   verify_training_design)
from research_runner_sizing import validate_plan_inputs, verified_job, INPUT_SCHEMA
import research_runner_sizing as sizing_runner
from research_runner_exact8 import SCHEMA

def row(identity, dd=25, growth=3, filled=100, active=20, green=60):
    return dict(id=identity,settings_sha256=identity,total_profit=100,max_dd_pct=dd,
                min_equity=500,positive_market_days_pct=green,median_weekly_return=.02,
                log_growth=growth,market_dates=['2026-06-22'],filled_baskets=filled,
                active_entry_days=active,blown=False,stop_outs=0)

class SizingResearchTests(unittest.TestCase):
    def test_300_unique_settings_change_only_volume_axes(self):
        base={'name':'GOD-X7','settings':{'lot_max':10,'entry_units':8,
            'rearm_max_times':0,'risk_per_basket_pct':20,'max_portfolio_risk_pct':0,
            'sentinel_nested':{'a':[1,2]}}}
        original=copy.deepcopy(base); rows=candidates(base)
        self.assertEqual(len(rows),300)
        self.assertEqual(len({r['settings_sha256'] for r in rows}),300)
        self.assertEqual(base,original)
        for r in rows:
            self.assertFalse(set(r['changes'])-NEW_FIELDS-{'lot_max'})
            self.assertEqual(r['preset']['settings']['lot_max'],5)
            self.assertEqual(r['preset']['settings']['risk_per_basket_pct'],20)
            self.assertEqual(r['preset']['settings']['lot_growth_basket_risk_pct'],0)
    def test_high_profit_does_not_override_activity_or_drawdown_gates(self):
        baseline=row('reference')
        rows=[row('good'),row('jackpot',growth=10,filled=20),row('crash',dd=80),
              row('inactive',active=2),row('few_positive',green=49)]
        result=select(rows,baseline)
        self.assertEqual([r['id'] for r in result['selected']],['good'])
    def test_pairing_and_preregistered_axis_coverage(self):
        rows=candidates({'name':'GOD-X7','settings':{'lot_max':10}})
        for a,b in zip(rows[::2],rows[1::2]):
            self.assertEqual(a['pair'],b['pair'])
            sa=a['preset']['settings'];sb=b['preset']['settings']
            self.assertTrue(all(sa[f]==0 for f in STRESS_FIELDS))
            self.assertTrue(1<=sum(sb[f]>0 for f in STRESS_FIELDS)<=3)
            self.assertEqual({k:v for k,v in sa.items() if k not in STRESS_FIELDS},
                             {k:v for k,v in sb.items() if k not in STRESS_FIELDS})
        self.assertEqual(Counter(len(r['active_stress_axes']) for r in rows[1::2]),{1:20,2:65,3:65})
        self.assertEqual(set(Counter(r['curve'] for r in rows[::2]).values()),{50})
        self.assertEqual(set(Counter(r['allocation'] for r in rows[::2]).values()),{30})
        self.assertTrue(set(Counter(r['anchor'] for r in rows[::2]).values())<={37,38})
    def test_distinct_low_dd_and_higher_growth_slots(self):
        rows=[row(f'low{i}',dd=10,growth=1+i*.01) for i in range(6)]
        rows += [row(f'growth{i}',dd=30,growth=4+i) for i in range(6)]
        picked=select(rows,row('ref'))['selected']
        self.assertEqual(len(picked),10)
        self.assertEqual(sum(r['id'].startswith('low') for r in picked),5)
        self.assertEqual(sum(r['id'].startswith('growth') for r in picked),5)
    def test_no_relaxation_and_no_partial_dates(self):
        bad=row('bad',dd=36); partial=row('partial');partial['market_dates']=[]
        result=select([bad,partial],row('ref'))
        self.assertEqual(result['status'],'NO_QUALIFYING_CANDIDATE')
        self.assertFalse(result['selected'])

    def test_real_export_field_names_include_rearm_and_still_open_entry_days(self):
        # This is the actual producer's Polish wrapper/field schema, with synthetic values.
        metrics = dict(trades=2,start_balance=600,end_equity=620)
        days=[dict(date='2026-06-22',start_equity=600,end_equity=620,real_dd_pct=2)]
        baskets=[dict(id=1,had_positions=True,first_open_ts=86400000,
                      warstwy=[dict(filled=True,fill_ts=3*86400000)]),
                 dict(id=2,had_positions=False,first_open_ts=0,warstwy=[])]
        ledger={'transakcje':[{'koszyk':1,'otwarcie_ms':86400000},
                              {'koszyk':1,'otwarcie_ms':2*86400000}]}
        result=describe('synthetic',metrics,{'dni':days},baskets,'sha',trades=ledger)
        self.assertEqual(result['active_entry_days'],3)
        self.assertEqual(result['filled_baskets'],1)
        with self.assertRaises(ValueError):
            describe('synthetic',metrics,{'dni':days},baskets,'sha')

    def test_failed_metrics_remain_serializable_and_cannot_be_selected(self):
        self.assertIsNone(weekly_median([]))
        self.assertIsNone(weekly_median([dict(date='2026-06-22',start_equity=0,end_equity=-2)]))
        bad=row('bad');bad.update(log_growth=None,median_weekly_return=None)
        result=select([bad],row('ref'))
        self.assertFalse(result['selected'])
        json.dumps(result,allow_nan=False)
        bad=row('bool');bad['max_dd_pct']=False
        self.assertFalse(select([bad],row('ref'))['selected'])

    def test_setting_number_spelling_is_semantic_but_booleans_remain_distinct(self):
        self.assertTrue(settings_equal({'a':[1,0.0]},{'a':[1.0,0]}))
        self.assertFalse(settings_equal({'a':[True]},{'a':[1]}))
        self.assertFalse(settings_equal({'a':[False]},{'a':[0.0]}))
        self.assertFalse(settings_equal({'a':1},{'a':1.01}))


class BindingTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve()
        self.study=self.root/'study';self.study.mkdir()
        self.source=self.root/'source';self.source.mkdir()
        (self.source/'synthetic.rs').write_text('synthetic build input',encoding='utf8')
        self.exe=self.root/'btp.exe';self.exe.write_bytes(b'not an executable; never launched')
        self.log=self.root/'build.log';self.log.write_text('synthetic PASS',encoding='utf8')
        self.manifest=self.root/'SOURCE_MANIFEST.json'
        write_new(self.manifest,[{'path':'synthetic.rs','sha256':pin(self.source/'synthetic.rs')['sha256']}])
        self.build=self.root/'BUILD_RECEIPT.json'
        write_new(self.build,dict(schema='conduit.sizing-instrument.v1',status='PASS',
            source_revalidated_after=True,source=str(self.source),source_manifest=pin(self.manifest),
            log=pin(self.log),binaries={'btp.exe':pin(self.exe)}))
        defaults={f:0. for f in NEW_FIELDS}
        defaults.update(lot_base='Balance',ea_enabled=False,t100={'enabled':False},
                        lot_growth_mode='Off',lot_max=10.,entry_units=8)
        base={'name':'GOD-X7','settings':defaults}
        self.baseline=self.root/'baseline.json';write_new(self.baseline,base)
        self.defaults=self.study/'DEFAULTS.json';write_new(self.defaults,defaults)
        self.materialized=self.study/'BASELINE_MATERIALIZED.json';write_new(self.materialized,base)
        self.prereg=self.study/'PREREGISTRATION.json';write_new(self.prereg,{'synthetic':True})
        self.ticks=self.root/'ticks.bin';self.ticks.write_bytes(b'synthetic ticks')
        self.signals=self.root/'signals.json';write_new(self.signals,[])
        self.configs=self.study/'configs';self.configs.mkdir()
        self.config=self.configs/'GOD-X7.json';write_new(self.config,base)
        self.results=self.study/'validation'/'results';self.results.mkdir(parents=True)
        argv=[str(self.exe),'--ticks',str(self.ticks),'--signals',str(self.signals),
              '--sweep',str(self.configs),'--out',str(self.results),'--from','2026-06-20',
              '--to','2026-08-01','--balance','600','--dump-trades','--sim-price-digits','2']
        self.reference=self.root/'reference.json'
        write_new(self.reference,dict(schema='conduit.g7.lot-growth.owner-reference.v1',
            status='PASS_REVALIDATED_REFERENCE_ONLY',cases=[{}, {'argv_exact':argv,'config':pin(self.baseline)}]))
        paths=[self.exe,self.build,self.reference,self.baseline,self.defaults,self.materialized,
               self.prereg,self.ticks,self.signals,self.config]
        paths += [Path(__file__).with_name(name) for name in ('research_runner.py',
                  'research_runner_exact8.py','research_runner_sizing.py',
                  'prepare_lot_growth_sweep.py','rank_lot_growth_sweep.py')]
        self.plan=dict(id='synthetic',output=str(self.study),progress_dir=str(self.study/'progress'),
            threads=24,inputs=[pin(p) for p in paths],
            source_revision=pin(self.build)['sha256'],
            qualification=dict(schema='conduit.sizing-plan-inputs.v1',build_receipt=pin(self.build),
                reference_binding=pin(self.reference),baseline=pin(self.baseline),defaults=pin(self.defaults),
                materialized_baseline=pin(self.materialized),preregistration=pin(self.prereg)),
            jobs=[dict(id='validation',threads=1,expected_candidates=1,result_dir=str(self.results),
                argv=recipe({'cases':[{}, {'argv_exact':argv}]},self.exe,self.configs,self.results,
                            '2026-08-01','2026-09-01',300))])
        self.plan_path=self.study/'PLAN.json';write_new(self.plan_path,self.plan)
        metrics=dict(trades=0,total_profit=0.,end_equity=300.,start_balance=300.)
        write_new(self.results/'wyniki_compound.json',{'GOD-X7':metrics})
        write_new(self.results/'GOD-X7_compound_dane.json',dict(preset='GOD-X7',tryb='compound',
            od='2026-08-01',do='2026-09-01',saldo_start=300.,metryki=metrics,
            dni=[dict(date='2026-08-03',start_equity=300,end_equity=300,real_dd_pct=0)]))
        write_new(self.results/'GOD-X7_compound_koszyki.json',[])
        digest=validate_plan_inputs(self.plan)
        self.receipt=dict(id='validation',status='complete',returncode=0,partial=False,validation_errors=[],
            plan_sha256=pin(self.plan_path)['sha256'],argv=self.plan['jobs'][0]['argv'],
            exe_sha256=pin(self.exe)['sha256'],source_revision=self.plan['source_revision'],threads=1,
            input_binding_schema=INPUT_SCHEMA,inputs_before_sha256=digest,inputs_after_sha256=digest,
            result_files=[str(self.results/'wyniki_compound.json')],
            result_sha256={'wyniki_compound.json':pin(self.results/'wyniki_compound.json')['sha256']},
            detail_artifacts_schema=SCHEMA,
            detail_artifacts={p.name:{k:pin(p)[k] for k in ('sha256','bytes')}
                              for p in self.results.glob('GOD-X7_*.json')})
        self.receipt_path=self.study/'validation'/'receipt.json';write_new(self.receipt_path,self.receipt)

    def overwrite(self,path,doc):
        path.write_text(json.dumps(doc),encoding='utf8')

    def test_build_and_august_results_are_valid_without_train_selection(self):
        verify_build(self.exe,self.build)
        rows,details=verified_results(self.study,'validation')
        self.assertEqual(set(rows),{'GOD-X7'})
        self.assertIsNone(details['GOD-X7']['transakcje'])
        with self.assertRaises((ValueError,KeyError)):
            verify_training_design(self.study)

    def test_wrong_executable_and_changed_frozen_source_rejected(self):
        other=self.root/'other.exe';other.write_bytes(b'other')
        with self.assertRaises(ValueError):verify_build(other,self.build)
        (self.source/'synthetic.rs').write_text('changed',encoding='utf8')
        with self.assertRaises(ValueError):validate_plan_inputs(self.plan)

    def test_failed_build_cannot_be_qualified_even_with_other_matching_pins(self):
        build=json.loads(self.build.read_text());build['status']='FAIL'
        self.overwrite(self.build,build)
        with self.assertRaises(ValueError):verify_build(self.exe,self.build)

    def test_mutated_input_cannot_receive_completion_proof(self):
        self.signals.write_text('[1]',encoding='utf8')
        with self.assertRaises(ValueError):validate_plan_inputs(self.plan)

    def test_receipt_must_bind_plan_argv_exe_and_both_input_checks(self):
        for field,value in [('plan_sha256','other'),('exe_sha256','other'),
                            ('inputs_after_sha256','other'),('inputs_before_sha256',None),
                            ('source_revision','other'),('argv',['other'])]:
            with self.subTest(field=field):
                altered=copy.deepcopy(self.receipt);altered[field]=value
                self.overwrite(self.receipt_path,altered)
                with self.assertRaises(ValueError):verified_job(self.study,'validation')
        self.overwrite(self.receipt_path,self.receipt)

    def test_actual_detail_window_cannot_be_substituted_with_repaired_detail_hash(self):
        path=self.results/'GOD-X7_compound_dane.json'
        detail=json.loads(path.read_text());detail['od']='2026-09-01'
        self.overwrite(path,detail)
        self.receipt['detail_artifacts'][path.name]={k:pin(path)[k] for k in ('sha256','bytes')}
        self.overwrite(self.receipt_path,self.receipt)
        with self.assertRaises(ValueError):verified_results(self.study,'validation')

    def test_unlisted_sweep_config_is_rejected_before_launch(self):
        write_new(self.configs/'extra.json',{})
        with self.assertRaises(ValueError):validate_plan_inputs(self.plan)

    def test_no_implicit_approximate_stride(self):
        ref={'cases':[{}, {'argv_exact':self.plan['jobs'][0]['argv']+['--quick-tick-stride','2']}]}
        with self.assertRaises(ValueError):
            recipe(ref,self.exe,self.configs,self.results,'2026-08-01','2026-09-01',300)

    def run_fake_process(self, mutate_input=False):
        payloads={p.name:p.read_bytes() for p in self.results.iterdir()}
        for path in self.results.iterdir():path.unlink()
        self.receipt_path.unlink()
        class Finished:
            pid=12345
            def poll(self):return 0
        def launch(argv, **kwargs):
            for name,data in payloads.items():(self.results/name).write_bytes(data)
            if mutate_input:self.signals.write_text('[1]',encoding='utf8')
            return Finished()
        with patch('sys.argv',['research_runner_sizing.py',str(self.plan_path)]), \
                patch('research_runner.subprocess.Popen',side_effect=launch):
            return sizing_runner.main()

    def test_actual_runner_completion_binds_before_after_and_details(self):
        self.assertEqual(self.run_fake_process(),0)
        rows,_=verified_results(self.study,'validation')
        self.assertEqual(set(rows),{'GOD-X7'})

    def test_actual_runner_refuses_complete_when_child_changes_an_input(self):
        self.assertEqual(self.run_fake_process(mutate_input=True),1)
        receipt=json.loads(self.receipt_path.read_text())
        self.assertEqual(receipt['status'],'failed')
        self.assertNotIn('inputs_after_sha256',receipt)
        self.assertTrue(any('Completion input verification failed' in x for x in receipt['validation_errors']))

    def test_real_preparer_layout_stays_valid_after_control_receipt_creation(self):
        output=self.root/'prepared'
        methods=self.root/'methods.md';methods.write_text('synthetic methods',encoding='utf8')
        defaults=json.loads(self.defaults.read_text())
        argv=['prepare_lot_growth_sweep.py','--exe',str(self.exe),'--baseline',str(self.baseline),
              '--reference-binding',str(self.reference),'--build-receipt',str(self.build),
              '--methods',str(methods),'--output',str(output)]
        with patch('sys.argv',argv),patch('prepare_lot_growth_sweep.subprocess.check_output',
                                         return_value=json.dumps(defaults).encode()):
            preparer.main()
        plan=json.loads((output/'PLAN.json').read_text())
        validate_plan_inputs(plan)
        write_new(output/'controls'/'receipt.json',{'status':'running'})
        validate_plan_inputs(plan)
        configdir=Path(plan['jobs'][0]['argv'][plan['jobs'][0]['argv'].index('--sweep')+1])
        self.assertEqual(configdir,output/'control_configs')
        self.assertEqual(len(list(configdir.glob('*.json'))),2)

if __name__=='__main__':unittest.main()
