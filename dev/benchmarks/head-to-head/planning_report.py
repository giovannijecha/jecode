"""Describe native Jecode edit batching alongside task outcomes, without ranking tests."""

import argparse
import json
from pathlib import Path

from analyze import jecode, codex, lines
from prepare import private_json


def edit_batches(run):
    responses = []
    for file in sorted((run/'home/sessions').glob('*/*/nodes/*.json')):
        for message in json.loads(file.read_text())['node']['messages']:
            if message['role'] != 'assistant': continue
            calls = [part for part in message['content'] if part.get('kind')=='tool_call']
            responses.append({'tools':[call['name'] for call in calls],
                              'paths':[call['input'].get('path') for call in calls],
                              'edits':sum(call['name'] in ('edit_file','write_file') for call in calls)})
    records = [row for file in (run/'home/diagnostics').glob('*.jsonl')
               for row in lines(file) if row.get('kind')=='request']
    # Diagnostics and canonical responses have the same order only on a complete
    # single turn with one checkpoint. Otherwise leave timing attribution unknown.
    aligned = (len(list((run/'home/sessions').glob('*/*/nodes/*.json')))==1
               and len(responses)==len(records) and all(row['outcome']=='completed' for row in records))
    groups = []; current = []
    for index,response in enumerate(responses):
        if response['edits'] and response['edits']==len(response['tools']):
            current.append(index)
        else:
            if len(current)>1: groups.append(current)
            current=[]
    if len(current)>1: groups.append(current)
    return {
        'responses':len(responses),'responsesWithEdits':sum(r['edits']>0 for r in responses),
        'responsesWithMultipleEdits':sum(r['edits']>1 for r in responses),
        'writeCalls':sum(r['edits'] for r in responses),'timingAlignmentVerified':aligned,
        'adjacentEditOnlyResponses':[
            {'requests':[i+1 for i in indices],
             'paths':[responses[i]['paths'] for i in indices],
             'providerMs':sum(records[i]['providerMs'] for i in indices) if aligned else None}
            for indices in groups],
    }


def main(root):
    output=[]
    for run in sorted((root/'runs').iterdir()):
        if not (run/'outcome.json').exists(): continue
        manifest=json.loads((run/'manifest.json').read_text())
        if manifest['preflight']: continue
        outcome=json.loads((run/'outcome.json').read_text())
        try: acceptance=json.loads((run/'acceptance.json').read_text())
        except (OSError,ValueError): acceptance={}
        evidence=jecode(run) if manifest['client']=='jecode' else codex(run)
        row={'run':run.name,'task':manifest.get('task','ledger'),'client':manifest['client'],
             'variant':manifest.get('variant','baseline'),'recovery':manifest.get('recovery',False),
             'status':outcome['status'],'elapsedMs':outcome.get('elapsedMs'),
             'passed':acceptance.get('passed'),'total':acceptance.get('total'),
             'requests':evidence.get('requests',evidence.get('usageEvents')),
             'outputTokens':evidence.get('outputTokens'),'evidence':evidence}
        if manifest['client']=='jecode': row['batching']=edit_batches(run)
        output.append(row)
    private_json(root/'planning-report.json',output)
    print(json.dumps(output,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    main(parser.parse_args().root.resolve())
