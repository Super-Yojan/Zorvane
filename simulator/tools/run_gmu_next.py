#!/usr/bin/env python3
"""Launch NEXT at GMU; retrieve an existing Maps key without printing it."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', default=os.environ.get('GOOGLE_CLOUD_PROJECT', 'glid-app'))
    parser.add_argument('--key-id', default=os.environ.get('GOOGLE_MAPS_KEY_ID'))
    parser.add_argument('--offline', action='store_true', help='Run competition without fetching Google scenery')
    parser.add_argument('--direct', action='store_true', help='Start competition immediately, skipping the menu')
    args = parser.parse_args()
    env = os.environ.copy()
    if not args.offline and not env.get('GOOGLE_MAPS_API_KEY', '').strip():
        gcloud = shutil.which('gcloud')
        fallback = Path.home() / 'Startups/google-cloud-sdk/bin/gcloud'
        if not gcloud and fallback.is_file():
            gcloud = str(fallback)
        if not gcloud:
            parser.error('Install/sign in to gcloud, or set GOOGLE_MAPS_API_KEY privately.')

        def cloud(*arguments):
            location = ['--location=global'] if arguments[0] == 'get-key-string' else []
            result = subprocess.run([gcloud, 'services', 'api-keys', *arguments,
                                     '--project=' + args.project, *location, '--quiet'],
                                    capture_output=True, text=True)
            if result.returncode:
                # Neither raw errors nor returned credentials may reach logs.
                parser.error('Google Cloud key lookup failed. Check account, project and API Keys permissions.')
            return result.stdout

        key_id = args.key_id
        if not key_id:
            try:
                keys = json.loads(cloud('list', '--format=json'))
                candidates = [key['name'].split('/')[-1] for key in keys
                              if any(target.get('service') == 'tile.googleapis.com'
                                     for target in key.get('restrictions', {}).get('apiTargets', []))]
            except (ValueError, KeyError, TypeError):
                parser.error('Could not read Google Cloud key metadata.')
            if len(candidates) != 1:
                parser.error('Choose an existing Map Tiles key with --key-id; automatic lookup needs exactly one matching key.')
            key_id = candidates[0]
        secret = cloud('get-key-string', key_id, '--format=value(keyString)').strip()
        if not secret:
            parser.error('Google Cloud returned no key.')
        env['GOOGLE_MAPS_API_KEY'] = secret
    if args.offline:
        env['TERRA_GMU_OFFLINE'] = '1'
        env.pop('GOOGLE_MAPS_API_KEY', None)
    if env.get('TT_TILES3D'):
        parser.error('Unset TT_TILES3D before launching this preset.')
    env['TERRA_NEXT_GMU'] = '1'
    env.setdefault('TERRA_ZENOH', '0')
    env['RUST_LOG'] = 'off'
    if args.direct:
        env['ZORVANE_MENU'] = '0'
    os.chdir(Path(__file__).resolve().parents[2])
    print('Starting NEXT - GMU Stadium. F6 switches overview/follow; WASD drives.', flush=True)
    os.execvpe('cargo', ['cargo', 'run', '-p', 'zorvane', '--bin', 'zorvane', '--features', 'gmu-viewer', '--locked'], env)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
