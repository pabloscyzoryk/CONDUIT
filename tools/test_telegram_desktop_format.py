import copy
import json
import unittest

import telegram_desktop_format as fmt


def sample(message=None):
    row={'_':'Message','id':7,'peer_id':{'_':'PeerChannel','channel_id':99},
         'date':1700000000,'message':'Synthetic text','entities':[]}
    if message:row.update(message)
    return {'channel':{'id':99,'title':'Synthetic Synergy','username':None,'broadcast':True},
            'messages':[row],'users':[]}


class FormatterTests(unittest.TestCase):
    def test_canonical_modes_preserve_text_and_do_not_invent_reception(self):
        doc=fmt.format_export(sample({'edit_date':1700000060,'reply_to':{'reply_to_msg_id':3}}))
        raw=fmt.desktop_json(doc).encode()
        publication=fmt.canonical_projection(doc,raw,'publication-final')
        edited=fmt.canonical_projection(doc,raw,'last-edit-final')
        p,e=publication['messages'][0],edited['messages'][0]
        self.assertEqual((p['ts'],p['edit_of']),(1700000000000,None))
        self.assertEqual((e['ts'],e['edit_of']),(1700000060000,7))
        self.assertEqual(p['text'],e['text'])
        self.assertEqual(p['reply_to'],3)
        self.assertEqual(publication['counts']['final_edit_before_edit_violations'],1)
        self.assertEqual(edited['counts']['final_edit_before_edit_violations'],0)
        self.assertNotIn('telegram_published_ts',p)
        self.assertNotIn('received_ts',p)
        self.assertEqual(publication['provenance']['source_kind'],'independent_telegram_api_snapshot_desktop_projection')

    def test_utf16_entity_and_desktop_trailing_empty_quirk(self):
        parts=fmt.text_parts('😀abc',[{'_':'MessageEntityBold','offset':2,'length':3}])
        self.assertEqual(parts,[{'type':'plain','text':'😀'},{'type':'bold','text':'abc'},{'type':'plain','text':''}])
        self.assertEqual(''.join(p['text'] for p in parts),'😀abc')

    def test_overlapping_entities_keep_original_order(self):
        parts=fmt.text_parts('abcd',[{'_':'MessageEntityBold','offset':0,'length':4},
                                     {'_':'MessageEntityItalic','offset':1,'length':2}])
        self.assertEqual(parts,[{'type':'bold','text':'abcd'}])

    def test_custom_emoji_requires_acquired_availability(self):
        entity=[{'_':'MessageEntityCustomEmoji','offset':0,'length':1,'document_id':123}]
        with self.assertRaisesRegex(fmt.Unsupported,'availability'):fmt.text_parts('x',entity)
        self.assertEqual(fmt.text_parts('x',entity,{'123':True})[0]['document_id'],'')
        self.assertEqual(fmt.text_parts('x',entity,{'123':False})[0]['document_id'],'(unavailable)')

    def test_unix_times_edit_reply_and_text_preserved(self):
        data=sample({'edit_date':1700000050,'reply_to':{'reply_to_msg_id':3}})
        before=copy.deepcopy(data)
        a=fmt.format_export(data,0)['messages'][0]
        b=fmt.format_export(data,120)['messages'][0]
        self.assertEqual(a['date_unixtime'],b['date_unixtime'])
        self.assertNotEqual(a['date'],b['date'])
        self.assertEqual(a['edited_unixtime'],'1700000050')
        self.assertEqual(a['reply_to_message_id'],3)
        self.assertEqual(a['text'],'Synthetic text')
        self.assertEqual(data,before)

    def test_no_media_photo_metadata_from_api(self):
        data=sample({'media':{'_':'MessageMediaPhoto','photo':{'sizes':[
            {'_':'PhotoSize','w':20,'h':30,'size':55},{'_':'PhotoSize','w':200,'h':300,'size':999}]}}})
        row=fmt.format_export(data)['messages'][0]
        self.assertEqual((row['photo'],row['photo_file_size'],row['width'],row['height']),
                         (fmt.NOT_INCLUDED,999,200,300))

    def test_service_and_rich_message(self):
        row=fmt.format_export(sample({'_':'MessageService','action':{'_':'MessageActionPinMessage'},
            'reply_to':{'reply_to_msg_id':3},'message':''}))['messages'][0]
        self.assertEqual((row['type'],row['action'],row['message_id']),('service','pin_message',3))

    def test_unknown_shapes_fail_instead_of_partial_export(self):
        for addition in [{'media':{'_':'FutureMedia'}},{'reply_markup':{'_':'Unknown'}},
                         {'entities':[{'_':'MessageEntityFuture','offset':0,'length':1}]},
                         {'_':'MessageService','action':{'_':'NewService'}},
                         {'rich_message':{'part':True,'blocks':[]}}]:
            with self.subTest(addition=addition), self.assertRaises(fmt.Unsupported):
                fmt.format_export(sample(addition))

    def test_json_single_indent_reaction_layout_and_no_final_newline(self):
        value={'reactions':[{'type':'emoji','count':2,'emoji':'x'}]}
        raw=fmt.desktop_json(value)
        self.assertEqual(raw,'{\n "reactions": [\n   {\n    "type": "emoji",\n    "count": 2,\n    "emoji": "x"\n   }\n  ]\n}')
        self.assertEqual(json.loads(raw),value)
        self.assertFalse(raw.endswith('\n'))

    def test_duplicate_or_foreign_identity_rejected(self):
        data=sample();data['messages']*=2
        with self.assertRaisesRegex(fmt.Unsupported,'duplicate'):fmt.format_export(data)
        data=sample();data['messages'][0]['peer_id']['channel_id']=100
        with self.assertRaisesRegex(fmt.Unsupported,'wrong_message_channel'):fmt.format_export(data)


if __name__=='__main__':unittest.main()
