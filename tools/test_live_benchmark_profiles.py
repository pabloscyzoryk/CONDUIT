import copy
import json
from pathlib import Path
import tempfile
import unittest

from live_benchmark_profiles import PROFILES, build_profile, generate, read_base


class LiveBenchmarkProfilesTests(unittest.TestCase):
    def fixture(self):
        return [{'ts':1800000000000+i*100000,'msg_id':i,'text':text,'kanal':'Synergy',
                 'reply_to':None if i==1 else 1,'telegram_published_ts':1800000000000+i*100000-500}
                for i,text in enumerate(['BUY GOLD @ 4000/3995\nTP 4010\nSL 3990',
                                         'MOVE SL TO 3992','RISK FREE 3995'],1)]

    def test_edit_recovery_preserves_content_and_time_without_duplicate_new_origins(self):
        base=self.fixture(); untouched=copy.deepcopy(base)
        profile=next(p for p in PROFILES if p.name=='edit_recovery')
        payload,manifest=build_profile(base,profile)
        self.assertEqual(base,untouched)
        self.assertFalse(payload['historical'])
        self.assertEqual(manifest['risk_free_artificially_delayed'],0)
        rows=payload['messages']
        self.assertEqual([r['receive_seq'] for r in rows],list(range(1,len(rows)+1)))
        self.assertEqual([r['ts'] for r in rows],sorted(r['ts'] for r in rows))
        for source in base[:2]:
            versions=[r for r in rows if r['msg_id']==source['msg_id']]
            self.assertEqual(versions[0]['edit_of'],source['msg_id'])
            self.assertEqual(versions[0]['ts'],source['ts'])
            self.assertEqual(sum(r['edit_of'] is None for r in versions),1)
            self.assertTrue(all(r['text']==source['text'] for r in versions))
            self.assertTrue(all(r['ts']>=source['ts'] for r in versions))
            self.assertTrue(all(r['telegram_published_ts']==source['telegram_published_ts'] for r in versions))

    def test_cannot_flatten_observed_revisions(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'source.json'
            base=self.fixture();base.append({**base[0],'edit_of':1,'ts':base[-1]['ts']+1})
            path.write_text(json.dumps({'messages':base}),encoding='utf-8')
            with self.assertRaisesRegex(ValueError,'observed revision'):
                read_base(path)

    def test_generated_inputs_have_identity_and_never_overwrite_previous_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=root/'source.json';output=root/'generated'
            source.write_text(json.dumps({'messages':self.fixture()}),encoding='utf-8')
            summary=generate(source,output,iter(['edit_recovery','reconnect']))
            self.assertEqual(set(summary['profiles']),{'edit_recovery','reconnect'})
            self.assertEqual(len(summary['source_sha256']),64)
            self.assertEqual(len(summary['generator_sha256']),64)
            with self.assertRaises(FileExistsError):
                generate(source,output,['edit_recovery'])


if __name__=='__main__':
    unittest.main()
