"""Predeclared training boundaries for this dated Synergy experiment."""
WINDOWS = {'historical': ['2026-06-20', '2026-08-17'],
           'observed': ['2026-08-05', '2026-08-17']}


def screening_contract(plan):
    protocol = plan.get('protocol', {})
    if (protocol.get('historical_training') != WINDOWS['historical']
            or protocol.get('observed_training') != WINDOWS['observed']
            or plan.get('metadata', {}).get('tryb_obliczen') != 'quick'
            or plan.get('metadata', {}).get('status_walidacji') != 'in_sample'):
        raise ValueError('Predeclared screening training contract required')
    for job in plan['jobs']:
        corpus = job.get('corpus')
        if (corpus not in WINDOWS or job.get('window') != WINDOWS[corpus]
                or job.get('deposit') != 600 or job.get('lot_cap') != 5):
            raise ValueError('Screening job differs from its training contract')
    return {'schema': 'conduit.training-boundaries.v1', 'stage': 'screening',
            'windows': WINDOWS, 'deposit': 600, 'lot_cap': 5,
            'later_outcomes_used': False}


def require_screening_selection(selection):
    contract = selection.get('screening_contract', {})
    if (contract.get('schema') != 'conduit.training-boundaries.v1'
            or contract.get('stage') != 'screening' or contract.get('windows') != WINDOWS
            or contract.get('deposit') != 600 or contract.get('lot_cap') != 5
            or contract.get('later_outcomes_used') is not False
            or selection.get('stage', 'screening') != 'screening'):
        raise ValueError('Verified screening training provenance required for neighbors')


def require_exact_training(plan):
    if (plan.get('metadata', {}).get('status_walidacji') != 'training_confirmation'
            or plan.get('metadata', {}).get('tryb_obliczen') != 'exact'
            or plan.get('protocol', {}).get('parameter_search') is not True):
        raise ValueError('Exact training stage required')
    expected = {key + '_exact_train': (key, value) for key, value in WINDOWS.items()}
    if {j['id'] for j in plan['jobs']} != set(expected):
        raise ValueError('Only the two predeclared training windows can narrow this queue')
    for job in plan['jobs']:
        corpus, window = expected[job['id']]
        if job.get('window') != {'from': window[0], 'to': window[1]}:
            raise ValueError('Exact training window differs from its predeclared boundary')
        argv = job.get('argv', [])
        for flag, value in [('--from', window[0]), ('--to', window[1]), ('--quick-tick-stride', '1')]:
            if argv.count(flag) != 1 or argv[argv.index(flag)+1] != value:
                raise ValueError('Exact training command differs from its declared window or stride')
