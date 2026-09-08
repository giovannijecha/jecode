"""Summarize fixed acceptance outcomes and comparable evidence without transcript content."""

import argparse
from collections import Counter
from datetime import datetime
import json
from pathlib import Path
import statistics


def read_json(file): return json.loads(file.read_text())


def lines(file):
    if not file.exists(): return []
    # Writers append complete JSONL records. A live read may see an unfinished tail.
    complete=file.read_text().split('\n')[:-1]
    return [json.loads(line) for line in complete if line.strip()]


def distribution(values):
    values=[value for value in values if isinstance(value,(int,float)) and not isinstance(value,bool)]
    if not values: return None
    ordered=sorted(values)
    return {'n':len(values),'sum':sum(values),'median':statistics.median(values),
            'min':ordered[0],'max':ordered[-1]}


def event_ns(event):
    stamp=event.get('fields',{}).get('event.timestamp')
    if not stamp: return None
    try: return int(datetime.fromisoformat(stamp.replace('Z','+00:00')).timestamp()*1e9)
    except ValueError: return None


def reported_errors(events):
    # The collector retains attribute names, never the private error message.
    # A response.completed event can carry an error instead of usage.
    errors = [event for event in events if 'error.message' in event.get('attributeKeys', [])]
    return {'count': len(errors), 'events': dict(Counter(
        event.get('fields', {}).get('event.name', 'unknown') for event in errors))}


def response_windows(events):
    """Pair serialized WebSocket sends/completions; reject ambiguous intervals."""
    pending=None
    durations=[]
    unmatched=0
    for event in sorted(events,key=lambda row:event_ns(row) or 0):
        fields=event['fields']
        if fields.get('event.name')=='codex.websocket_request':
            if pending is not None: unmatched+=1
            pending=event_ns(event)
        elif fields.get('event.name')=='codex.sse_event' and fields.get('event.kind')=='response.completed':
            if pending is None:
                unmatched+=1
            else:
                durations.append((event_ns(event)-pending)/1e6)
                pending=None
    unmatched+=int(pending is not None)
    return {'unmatched':unmatched,'milliseconds':distribution(durations) if unmatched==0 else None}


def jecode(run):
    records=[]
    for file in (run/'home/diagnostics').glob('*.jsonl'): records.extend(lines(file))
    requests=[row for row in records if row.get('kind')=='request']
    nodes=[read_json(file) for file in (run/'home/sessions').glob('*/*/nodes/*.json')]
    calls=[]; results=[]; tools=[]
    for saved in nodes:
        node=saved['node']
        for message in node['messages']:
            for content in message.get('content',[]):
                if content.get('kind')=='tool_call': calls.append(content)
                if content.get('kind')=='tool_result': results.append(content)
        tools.extend(block for block in node['blocks'] if block.get('kind')=='tool')
    reported=sum(r.get('reportedInputTokens',0) for r in requests)
    cached=sum(r.get('cachedInputTokens',0) for r in requests)
    return {'requests':len(requests),'outcomes':dict(Counter(r['outcome'] for r in requests)),
            'timings':{key:distribution([r.get(key) for r in requests]) for key in ('preparationMs','providerMs','firstEventMs')},
            'transport':dict(Counter(r.get('transport') for r in requests)),
            'incremental':sum(r.get('incremental',False) for r in requests),
            'reused':sum(r.get('reused',False) for r in requests),
            'fallbacks':sum(r.get('fallback',False) for r in requests),
            'compactions':len([r for r in records if r.get('kind')=='compaction']),
            'clippedResults':sum(r.get('clippedResults',0) for r in requests),
            'transportFailures':dict(Counter(r['transportFailure'] for r in requests if 'transportFailure' in r)),
            'requestsWithUsage':sum('outputTokens' in r for r in requests),
            'initialFirstEventMs':requests[0].get('firstEventMs') if requests else None,
            'initialPreparationMs':requests[0].get('preparationMs') if requests else None,
            'requestBytes':sum(r.get('requestBytes',0) for r in requests),
            'reportedInputTokens':reported,'cachedInputTokens':cached,
            'cachedFraction':cached/reported if reported else None,
            'outputTokens':sum(r.get('outputTokens',0) for r in requests),
            'reasoningTokens':sum(r.get('reasoningTokens',0) for r in requests),
            'toolCalls':len(calls),'toolResults':len(results),
            'tools':dict(Counter(c['name'] for c in calls)),
            'unmatchedCalls':len(set(c['id'] for c in calls)-set(r['id'] for r in results)),
            'toolErrors':sum(r.get('isError',False) for r in results),
            'recorderEnd':[r for r in records if r.get('kind')=='end']}


def task_events(events, started):
    """A pre-submission send may finish after paste; do not count it as task usage."""
    pending = None
    has_pending = False
    ambiguous = False
    excluded = set()
    for event in sorted(events, key=lambda row: event_ns(row) or 0):
        fields = event['fields']
        stamp = event_ns(event)
        if fields.get('event.name') == 'codex.websocket_request':
            ambiguous = ambiguous or has_pending
            pending = stamp
            has_pending = True
        elif fields.get('event.name') == 'codex.sse_event' and fields.get('event.kind') == 'response.completed':
            if not ambiguous and pending is not None and pending < started <= (stamp or 0):
                excluded.add(id(event))
            pending = None
            has_pending = False
            ambiguous = False
    return ([event for event in events if (event_ns(event) or 0) >= started and id(event) not in excluded],
            len(excluded))


def codex(run):
    started=read_json(run/'start.json')['atNs']
    all_events=lines(run/'codex-otel.jsonl')
    located, pre_submission = task_events(all_events, started)
    events=[r['fields'] for r in located]
    kinds=Counter(r.get('event.name') for r in events)
    stream=Counter(r.get('event.kind') for r in events if r.get('event.name')=='codex.sse_event')
    usage=[r for r in events if r.get('event.name')=='codex.sse_event' and r.get('output_token_count') is not None]
    def total(key):
        values=[int(r[key]) for r in usage if r.get(key) is not None]
        return sum(values) if values else None
    model_cache=read_json(run/'home/models_cache.json') if (run/'home/models_cache.json').exists() else {}
    models=model_cache.get('models',[])
    selected=[{key:model.get(key) for key in ('slug','context_window','effective_context_window_percent','default_reasoning_level','supported_reasoning_levels','service_tiers','default_service_tier')}
              for model in models if model.get('slug')=='gpt-6-astra']
    items=[]
    rollout_usage=[]
    for file in (run/'home/sessions').rglob('*.jsonl'):
        for record in lines(file):
            payload=record.get('payload',{})
            if record.get('type')=='response_item': items.append(payload)
            if record.get('type')=='event_msg' and payload.get('type')=='token_count':
                value=(payload.get('info') or {}).get('total_token_usage')
                if value is not None: rollout_usage.append(value)
    calls=[r for r in items if r.get('type') in ('function_call','custom_tool_call')]
    results=[r for r in items if r.get('type') in ('function_call_output','custom_tool_call_output')]
    return {'eventCounts':dict(kinds),'streamKinds':dict(stream),'modelMetadata':selected,
            'reportedErrors':reported_errors(located),
            'preSubmissionCompletions':pre_submission,
            'modelToolCalls':len(calls),'modelToolResults':len(results),
            'modelTools':dict(Counter(r.get('name') for r in calls)),
            'unmatchedCalls':len(set(r.get('call_id') for r in calls)-set(r.get('call_id') for r in results)),
            'usageEvents':len(usage),'reportedInputTokens':total('input_token_count'),
            'usageMatchesRollout':all(rollout_usage[-1].get(target)==total(source) for target,source in (
                ('input_tokens','input_token_count'),('output_tokens','output_token_count'),
                ('cached_input_tokens','cached_token_count'),('reasoning_output_tokens','reasoning_token_count'),
            )) if rollout_usage and usage else None,
            'outputTokens':total('output_token_count'),'cachedInputTokens':total('cached_token_count'),
            'reasoningTokens':total('reasoning_token_count'),
            'turnTtftMs':distribution([int(r['duration_ms']) for r in events if r.get('event.name')=='codex.turn_ttft' and 'duration_ms' in r]),
            'responseWindows':response_windows(located),
            'reused':sum(r.get('auth.connection_reused') is True for r in events if r.get('event.name')=='codex.websocket_request'),
            'websocketSendMs':distribution([int(r['duration_ms']) for r in events if r.get('event.name')=='codex.websocket_request' and 'duration_ms' in r]),
            'toolEventMsIncludingNested':distribution([int(r['duration_ms']) for r in events if r.get('event.name')=='codex.tool_result' and 'duration_ms' in r]),
            'unlocatedEvents':sum(event_ns(r) is None for r in all_events),
            'unlocatedEventNames':dict(Counter(r['fields'].get('event.name') for r in all_events if event_ns(r) is None)),
            'tools':dict(Counter(r.get('tool_name',r.get('tool')) for r in events if r.get('event.name')=='codex.tool_result'))}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    parser.add_argument('--live',action='store_true')
    args=parser.parse_args()
    output=[]
    for run in sorted((args.root/'runs').iterdir()):
        if not (run/'outcome.json').exists() and not (args.live and (run/'start.json').exists()): continue
        manifest=read_json(run/'manifest.json')
        if manifest['preflight']: continue
        outcome=read_json(run/'outcome.json') if (run/'outcome.json').exists() else {'status':'running'}
        acceptance_file=run/'acceptance-recheck.json' if (run/'acceptance-recheck.json').exists() else run/'acceptance.json'
        try: acceptance=read_json(acceptance_file)
        except (OSError,ValueError): acceptance={}
        result={'run':run.name,'client':manifest['client'],'task':manifest.get('task','ledger'),
                'variant':manifest.get('variant','baseline'),'recovery':manifest.get('recovery',False),'outcome':outcome,
                'acceptance':{key:acceptance.get(key) for key in ('passed','total')},
                'failedChecks':[r['name'] for r in acceptance.get('results',[]) if not r['passed']],
                'evidence':jecode(run) if manifest['client']=='jecode' else codex(run)}
        output.append(result)
    report=args.root/'comparison.json'
    report.write_text(json.dumps(output,indent=2)+'\n')
    print(json.dumps(output,indent=2))


if __name__=='__main__': main()
