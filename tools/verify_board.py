"""Upload a deck and verify acknowledged refreshes through the local API."""
import argparse
import json
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from pathlib import Path


def status(base):
    with urlopen(base + '/api/status', timeout=5) as response:
        assert response.headers.get_content_type() == 'application/json'
        return json.load(response)


def upload(base, payload):
    request = Request(base + '/api/japanese/daily', data=payload,
                      headers={'Content-Type': 'application/json'}, method='POST')
    with urlopen(request, timeout=30) as response:
        assert response.headers.get_content_type() == 'application/json'
        return json.load(response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('url')
    parser.add_argument('deck', type=Path)
    parser.add_argument('--seconds', type=int, default=200,
                        help='Observe at least 200 seconds to include a periodic clean refresh')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    base = args.url.rstrip('/')
    payload = args.deck.read_bytes()
    deck = json.loads(payload)
    result = upload(base, payload)
    assert result['saved'] and result['cards'] == len(deck['cards']), result
    print('Upload:', result, flush=True)
    revision = status(base)['deck_revision']
    try:
        upload(base, payload + b' trailing garbage')
    except HTTPError as error:
        assert error.code == 400, error.code
        print('Malformed upload rejected:', error.read().decode(), flush=True)
    else:
        raise AssertionError('Malformed deck accepted')
    assert status(base)['deck_revision'] == revision
    samples = []
    end = time.monotonic() + args.seconds
    last_position = None
    while time.monotonic() < end:
        sample = status(base)
        assert sample['cards'] == len(deck['cards']), sample
        assert not sample['error'] and sample['refresh_failures'] == 0, sample
        position = sample['displayed_position']
        if sample['deck_revision'] == revision and sample['current'].startswith('japanese:'):
            assert sample['display_ready'], sample
            assert sample['current'] == 'japanese:' + deck['cards'][position - 1]['id'], sample
        samples.append(sample)
        if position != last_position:
            print('Refresh:', json.dumps(sample), flush=True)
            last_position = position
        time.sleep(2)
    assert any(s['display_ready'] and s['current'].startswith('japanese:') for s in samples)
    if args.seconds >= 70:
        assert len({s['displayed_position'] for s in samples if s['displayed_position']}) >= 2
        assert any(s['profile']['partial_refresh_ms'] > 0 for s in samples)
    if args.seconds >= 200:
        # Three minutes elapse from the initial clean refresh. A subsequent
        # clean refresh can have the same duration, so identify it when the
        # current refresh differs from the previously measured partial refresh.
        assert any(s['displayed_position'] >= 4 and
                   s['profile']['refresh_ms'] == s['profile']['clean_refresh_ms'] and
                   s['profile']['refresh_ms'] != s['profile']['partial_refresh_ms'] for s in samples)
    if args.output:
        args.output.write_text(json.dumps(samples, indent=2) + '\n')
    print('Board API and acknowledged refresh checks passed.', flush=True)


if __name__ == '__main__':
    main()
