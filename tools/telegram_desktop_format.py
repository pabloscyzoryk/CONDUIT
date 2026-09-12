"""Offline Desktop-style JSON formatting of independently acquired Telegram TL data.

No reference document is read. Formatting follows the public Telegram Desktop
JSON writer and export text parser. Supported data is explicit; unknown media,
service actions and rich blocks fail instead of silently dropping their fields.
Snapshot metadata remains the current server state, not an older archive.
"""
from __future__ import annotations
import argparse
import base64
import datetime as dt
import hashlib
import json
from pathlib import Path

NOT_INCLUDED = '(File not included. Change data exporting settings to download.)'
ENTITIES = {
    'Unknown':'unknown','Mention':'mention','Hashtag':'hashtag','BotCommand':'bot_command',
    'Url':'link','Email':'email','Bold':'bold','Italic':'italic','Code':'code','Pre':'pre',
    'TextUrl':'text_link','MentionName':'mention_name','Phone':'phone','Cashtag':'cashtag',
    'Underline':'underline','Strike':'strikethrough','Blockquote':'blockquote','BankCard':'bank_card',
    'Spoiler':'spoiler','CustomEmoji':'custom_emoji',
}


class Unsupported(ValueError):
    pass


def text_parts(text, entities, custom_emoji_status=None):
    """Telegram offsets use UTF-16 units; preserve plain text exactly.

    Desktop walks the original entity order and skips overlaps, then appends
    the remaining QString suffix. Do not sort or flatten away the text itself.
    """
    encoded = text.encode('utf-16-le')
    def piece(start, length=None):
        end = len(encoded) if length is None else 2*(start+length)
        return encoded[2*start:end].decode('utf-16-le')
    result, offset = [], 0
    for entity in entities or []:
        start, length = entity['offset'], entity['length']
        if type(start) is not int or type(length) is not int:
            raise Unsupported('noninteger_entity_offset')
        if start < offset or length <= 0 or start+length > len(text.encode('utf-8')):
            continue
        if start > offset: result.append({'type':'plain','text':piece(offset,start-offset)})
        name = entity['_'].removeprefix('MessageEntity')
        if name not in ENTITIES: raise Unsupported('unsupported_entity_type')
        part = {'type':ENTITIES[name], 'text':piece(start,length)}
        if name=='Pre': part['language']=entity.get('language','')
        elif name=='TextUrl': part['href']=entity['url']
        elif name=='MentionName': part['user_id']=entity['user_id']
        elif name=='CustomEmoji':
            part['document_id']=emoji_path(entity['document_id'], custom_emoji_status)
        elif name=='Blockquote': part['collapsed']=bool(entity.get('collapsed'))
        result.append(part)
        offset=start+length
    # Desktop's ParseText final addTextPart uses the original UTF-8 byte size
    # with a QString::mid (UTF-16) offset. For a trailing entity after non-ASCII
    # text this can intentionally append an empty plain part. Preserve this
    # serialization quirk rather than invent a cleaner, different text array.
    byte_size=len(text.encode('utf-8'))
    if offset < byte_size: result.append({'type':'plain','text':piece(offset,byte_size-offset)})
    if ''.join(p['text'] for p in result) != text:
        raise Unsupported('entity_projection_lost_text')
    return result


def emoji_path(document_id, status):
    # Desktop resolves document availability, then its no-media policy emits
    # an empty path. Missing documents retain the official unavailable marker.
    # A message entity alone does not establish document availability.
    key=str(document_id)
    if status is None or type(status.get(key)) is not bool:
        raise Unsupported('custom_emoji_availability_not_acquired')
    return '' if status[key] else '(unavailable)'


def displayed_text(parts):
    if not parts: return ''
    if len(parts)==1 and parts[0]['type']=='plain': return parts[0]['text']
    return [p['text'] if p['type']=='plain' else p for p in parts]


def epoch(value):
    if type(value) is int: return value
    if isinstance(value,str):
        parsed=dt.datetime.fromisoformat(value.replace('Z','+00:00'))
        if parsed.tzinfo is None: raise Unsupported('raw_timestamp_missing_timezone')
        return int(parsed.timestamp())
    raise Unsupported('invalid_raw_timestamp')


def photo_size(sizes):
    choices=[s for s in sizes or [] if s.get('_') not in ['PhotoSizeEmpty','PhotoStrippedSize','PhotoPathSize'] and s.get('w',0)>0 and s.get('h',0)>0]
    if not choices: return None
    s=max(choices,key=lambda x:x['w']*x['h'])
    if s['_']=='PhotoSizeProgressive': size=(s.get('sizes') or [0])[-1]
    elif s['_']=='PhotoCachedSize':
        raw=s.get('bytes',{})
        size=len(base64.b64decode(raw['bytes_base64'])) if isinstance(raw,dict) else len(raw)
    elif s['_']=='PhotoSize': size=s['size']
    else: raise Unsupported('unsupported_photo_size')
    return {'bytes':size,'width':s['w'],'height':s['h']}


def rich_text(value):
    kind=value['_']
    if kind=='TextEmpty':return {'type':'empty'}
    if kind=='TextPlain':return {'type':'plain','text':value['text']}
    if kind=='TextConcat':return {'type':'concat','text':[rich_text(x) for x in value['texts']]}
    names={'TextBold':'bold','TextItalic':'italic','TextUnderline':'underline','TextStrike':'strikethrough','TextFixed':'fixed','TextSubscript':'subscript','TextSuperscript':'superscript','TextMarked':'marked'}
    if kind in names:return {'type':names[kind],'text':rich_text(value['text'])}
    raise Unsupported('unsupported_rich_text')


def rich_message(value):
    if value.get('part'):
        raise Unsupported('partial_rich_message_not_complete')
    if value.get('photos') or value.get('documents'):
        raise Unsupported('rich_media_not_supported')
    blocks=[]
    names={'PageBlockParagraph':'paragraph','PageBlockFooter':'footer'}
    for b in value['blocks']:
        if b['_'] not in names:raise Unsupported('unsupported_rich_block')
        blocks.append({'type':names[b['_']],'text':rich_text(b['text'])})
    return {'rtl':bool(value.get('rtl')),'part':bool(value.get('part')),'blocks':blocks}


def format_export(snapshot, utc_offset_minutes=120):
    if type(utc_offset_minutes) is not int or abs(utc_offset_minutes)>14*60:
        raise Unsupported('invalid_display_utc_offset')
    channel=snapshot['channel']
    cid=channel['id']
    names={('channel',cid):channel['title']}
    usernames={}
    for user in snapshot.get('users',[]):
        names[('user',user['id'])]=' '.join(x for x in [user.get('first_name'),user.get('last_name')] if x) or None
        usernames[user['id']]=user.get('username')
    def peer(value):
        if not value:return ('channel',cid)
        for tag,key,prefix in [('PeerChannel','channel_id','channel'),('PeerChat','chat_id','chat'),('PeerUser','user_id','user')]:
            if value.get('_')==tag:return (prefix,value[key])
        raise Unsupported('unsupported_sender_peer')
    def push_from(out,raw,label='from'):
        key=peer(raw.get('from_id') or raw.get('peer_id'))
        out[label]=names.get(key)
        out[label+'_id']=key[0]+str(key[1])
    def date(ts):return (dt.datetime.fromtimestamp(epoch(ts),dt.timezone.utc)+dt.timedelta(minutes=utc_offset_minutes)).strftime('%Y-%m-%dT%H:%M:%S')
    rows=[]
    seen=set()
    for raw in snapshot['messages']:
        if raw['id'] in seen:raise Unsupported('duplicate_message')
        seen.add(raw['id'])
        if peer(raw.get('peer_id'))!=('channel',cid):raise Unsupported('wrong_message_channel')
        service=raw['_']=='MessageService'
        if raw['_'] not in ['Message','MessageService']:raise Unsupported('unsupported_message_constructor')
        out={'id':raw['id'],'type':'service' if service else 'message','date':date(raw['date']),'date_unixtime':str(epoch(raw['date']))}
        if raw.get('edit_date'):
            out.update(edited=date(raw['edit_date']),edited_unixtime=str(epoch(raw['edit_date'])))
        reply=raw.get('reply_to') or {}
        if service:
            push_from(out,raw,'actor')
            action=raw['action']
            if action['_']=='MessageActionChannelCreate':out.update(action='create_channel',title=action['title'])
            elif action['_']=='MessageActionPinMessage':
                out['action']='pin_message'
                if reply.get('reply_to_msg_id'):out['message_id']=reply['reply_to_msg_id']
            else:raise Unsupported('unsupported_service_action')
        else:
            push_from(out,raw)
            if raw.get('post_author'):out['author']=raw['post_author']
            forward=raw.get('fwd_from') or {}
            if forward.get('from_id'):
                key=peer(forward['from_id']);out.update(forwarded_from=names.get(key),forwarded_from_id=key[0]+str(key[1]))
            elif forward.get('from_name'):out['forwarded_from']=forward['from_name']
            if reply.get('reply_to_msg_id'):
                out['reply_to_message_id']=reply['reply_to_msg_id']
                if reply.get('reply_to_peer_id') and peer(reply['reply_to_peer_id'])!=('channel',cid):
                    key=peer(reply['reply_to_peer_id']);out['reply_to_peer_id']=key[0]+str(key[1])
            if raw.get('via_bot_id') and usernames.get(raw['via_bot_id']):out['via_bot']='@'+usernames[raw['via_bot_id']]
        media=raw.get('media') or {}
        kind=media.get('_')
        if kind=='MessageMediaPhoto':
            image=photo_size((media.get('photo') or {}).get('sizes'))
            if image is None:raise Unsupported('photo_metadata_unavailable')
            out.update(photo=NOT_INCLUDED,photo_file_size=image['bytes'],width=image['width'],height=image['height'])
        elif kind=='MessageMediaDocument':
            doc=media.get('document') or {}
            if doc.get('_')!='Document':raise Unsupported('document_unavailable')
            attrs={a['_']:a for a in doc.get('attributes',[])}
            out['file']=NOT_INCLUDED
            filename=(attrs.get('DocumentAttributeFilename') or {}).get('file_name')
            if filename:out['file_name']=filename
            out['file_size']=doc['size']
            thumb=photo_size(doc.get('thumbs'))
            if thumb:out.update(thumbnail=NOT_INCLUDED,thumbnail_file_size=thumb['bytes'])
            video=attrs.get('DocumentAttributeVideo')
            audio=attrs.get('DocumentAttributeAudio')
            if attrs.get('DocumentAttributeSticker'):
                out['media_type']='sticker'
                if attrs['DocumentAttributeSticker'].get('alt'):out['sticker_emoji']=attrs['DocumentAttributeSticker']['alt']
            elif video and video.get('round_message'):out['media_type']='video_message'
            elif audio and audio.get('voice'):out['media_type']='voice_message'
            elif attrs.get('DocumentAttributeAnimated'):out['media_type']='animation'
            elif video:out['media_type']='video_file'
            elif audio:
                out['media_type']='audio_file'
                if audio.get('performer'):out['performer']=audio['performer']
                if audio.get('title'):out['title']=audio['title']
            out['mime_type']=doc['mime_type']
            timing=video or audio
            if timing and timing.get('duration'):out['duration_seconds']=int(timing['duration'])
            dims=video or attrs.get('DocumentAttributeImageSize')
            if dims and dims.get('w') and dims.get('h'):out.update(width=dims['w'],height=dims['h'])
        elif kind not in (None,'MessageMediaEmpty','MessageMediaWebPage'):
            raise Unsupported('unsupported_media_constructor')
        if media.get('spoiler'):out['media_spoiler']=True
        if media.get('ttl_seconds'):out['self_destruct_period_seconds']=media['ttl_seconds']
        if raw.get('rich_message'):out['rich_message']=rich_message(raw['rich_message'])
        else:
            parts=text_parts(raw.get('message',''),raw.get('entities'),snapshot.get('custom_emoji_status'))
            out.update(text=displayed_text(parts),text_entities=parts)
        if raw.get('reply_markup'):raise Unsupported('inline_markup_not_supported')
        reactions=[]
        for reaction in (raw.get('reactions') or {}).get('results') or []:
            r=reaction['reaction'];types={'ReactionEmoji':'emoji','ReactionCustomEmoji':'custom_emoji','ReactionPaid':'paid','ReactionEmpty':'empty'}
            if r['_'] not in types:raise Unsupported('unsupported_reaction')
            result={'type':types[r['_']],'count':reaction['count']}
            if r['_']=='ReactionEmoji':result['emoji']=r['emoticon']
            elif r['_']=='ReactionCustomEmoji':result['document_id']=emoji_path(r['document_id'],snapshot.get('custom_emoji_status'))
            reactions.append(result)
        if (raw.get('reactions') or {}).get('recent_reactions'):
            raise Unsupported('recent_reaction_people_not_supported')
        if reactions:out['reactions']=reactions
        rows.append(out)
    rows.sort(key=lambda r:r['id'])
    return {'name':channel['title'],'type':('public_channel' if channel.get('username') else 'private_channel') if channel.get('broadcast') else ('public_supergroup' if channel.get('username') else 'private_group'),'id':cid,'messages':rows}


def desktop_json(value, depth=0):
    """Single-space Desktop layout, including its extra reaction indentation."""
    if isinstance(value,dict):
        parts=[]
        for key,item in value.items():
            child_depth=depth+1+(1 if key=='reactions' else 0)
            parts.append(' '*(depth+1)+json.dumps(key,ensure_ascii=False)+': '+desktop_json(item,child_depth))
        return '{\n'+',\n'.join(parts)+'\n'+' '*depth+'}'
    if isinstance(value,list):
        if not value:return '[]'
        return '[\n'+',\n'.join(' '*(depth+1)+desktop_json(item,depth+1) for item in value)+'\n'+' '*depth+']'
    return json.dumps(value,ensure_ascii=False,allow_nan=False).replace('\u2028','\\u2028').replace('\u2029','\\u2029')


def canonical_projection(document, source_bytes, time_mode):
    """Explicit historical loader projection, never an observed NEW/EDIT log.

    Matches the historical converter's consumed fields and deterministic source
    order. Its provenance identifies this API snapshot, not a Desktop download.
    """
    if time_mode not in ('publication-final', 'last-edit-final'):
        raise Unsupported('unknown_canonical_time_mode')
    def text_of(value):
        if isinstance(value,str):return value
        if isinstance(value,list):return ''.join(text_of(v) for v in value)
        if isinstance(value,dict):return text_of(value.get('text',''))
        return ''
    def number(value):
        if isinstance(value,bool):raise Unsupported('boolean_canonical_identity_or_time')
        if type(value) is int:return value
        if isinstance(value,str) and value.lstrip('-').isdigit():return int(value)
        raise Unsupported('invalid_canonical_identity_or_time')
    def milliseconds(value):
        raw=number(value)
        return raw*1000 if abs(raw)<1_000_000_000_000 else raw
    messages=[]
    empty=edited_count=violations=0
    for index,raw in enumerate(document['messages']):
        text=text_of(raw.get('text',''))
        if not text.strip():empty+=1;continue
        mid=number(raw['id'])
        published=milliseconds(raw['date_unixtime'])
        edited=milliseconds(raw['edited_unixtime']) if raw.get('edited_unixtime') not in (None,'',0,'0') else None
        was_edited=edited is not None and edited>published
        edited_count+=was_edited
        ts=published if time_mode=='publication-final' or not was_edited else edited
        violations+=bool(was_edited and ts<edited)
        reply=number(raw['reply_to_message_id']) if raw.get('reply_to_message_id') not in (None,'') else None
        messages.append({'ts':ts,'msg_id':mid,'reply_to':reply,
            'edit_of':mid if was_edited and time_mode=='last-edit-final' else None,
            'text':text,'kanal':'Synergy','provenance':{
                'source_index':index,'source_type':raw.get('type'),
                'telegram_published_ms':published,'telegram_edited_ms':edited,
                'version':'final_only' if was_edited else 'original_unedited',
                'limitation':'pre_edit_text_unavailable' if was_edited else None}})
    messages.sort(key=lambda m:(m['ts'],m['provenance']['source_index']))
    return {'schema':'conduit.raw-message-stream.v1','provenance':{
        'source_kind':'independent_telegram_api_snapshot_desktop_projection',
        'source_sha256':hashlib.sha256(source_bytes).hexdigest(),'source_bytes':len(source_bytes),
        'channel':'Synergy','time_mode':time_mode,
        'ts_semantics':('telegram_publication_time_WITH_FINAL_TEXT; optimistic_approximation_NOT_causal_NOT_receive_time'
            if time_mode=='publication-final' else
            'telegram_edited_time_for_known_final_edit_else_publish_time; NOT_local_conduit_receive_time'),
        'limitations':['Current server version only; deleted messages and prior edit bodies are unavailable.',
            'No local reception time, measured latency or receive sequence was synthesized.',
            'Publication-final can expose later edited text before its edit; neither mode is a complete causal chronicle.']},
        'counts':{'source_records':len(document['messages']),'nonempty_messages':len(messages),
            'empty_or_media_only_skipped':empty,'edited_final_only':edited_count,
            'final_edit_before_edit_violations':violations},'messages':messages}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('snapshot',type=Path)
    p.add_argument('output_directory',type=Path)
    p.add_argument('--utc-offset-minutes',type=int,required=True,help='Explicit displayed local-time offset, measured from the reference options; Unix times are unchanged.')
    args=p.parse_args()
    args.output_directory.mkdir(parents=True,exist_ok=False)
    doc=format_export(json.loads(args.snapshot.read_text(encoding='utf-8')),args.utc_offset_minutes)
    output=args.output_directory/'result.json'
    raw=desktop_json(doc).encode('utf-8')
    with output.open('xb') as handle:handle.write(raw)
    print(json.dumps({'status':'FORMATTED','records':len(doc['messages']),'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),'historical_byte_parity_verified':False}))


if __name__=='__main__':main()
