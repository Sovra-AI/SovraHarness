# Runs only on the authenticated Pi. Each deployment gets a new directory.
import json
import os
import pathlib
import signal
import subprocess
import tempfile
import time


def process_identity(pid):
    try:
        fields = pathlib.Path('/proc/%d/stat' % pid).read_text().rsplit(')', 1)[1].split()
        return fields[19]
    except (OSError, ValueError, IndexError):
        return None


def stop_previous(root):
    try:
        active = json.loads((root / 'active.json').read_text())
        pid = active['pid']
        release = pathlib.Path(active['directory'])
        if not isinstance(pid, int) or pid <= 1 or release.parent != root:
            return
        if not active.get('identity') or process_identity(pid) != active['identity']:
            return
        if pathlib.Path('/proc/%d/cwd' % pid).resolve() != release or os.getpgid(pid) != pid:
            return
        os.killpg(pid, signal.SIGTERM)
        for attempt in range(20):
            if process_identity(pid) != active['identity']:
                return
            time.sleep(0.1)
        os.killpg(pid, signal.SIGKILL)
    except (OSError, ValueError, KeyError, TypeError):
        return


def deploy(payload, model, root):
    if model != payload['model']:
        raise ValueError('The connected device changed. Connect again before deploying.')
    root.mkdir(parents=True, exist_ok=True)
    release = pathlib.Path(tempfile.mkdtemp(prefix='pi-', dir=str(root)))
    project = payload['project']
    for name, content in project['files'].items():
        parts = name.split('/')
        if any(not part or part.startswith('.') or not all(c.isascii() and (c.isalnum() or c in '_.-') for c in part) for part in parts):
            raise ValueError('Invalid source path')
        destination = release.joinpath(*parts)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content, encoding='utf-8')
    kind = project['kind']
    if kind == 'python':
        command = ['python3', '-u', 'main.py']
    elif kind == 'node':
        command = ['node', 'main.js']
    elif kind == 'website':
        command = ['python3', '-m', 'http.server', '8080', '--bind', '0.0.0.0']
    else:
        raise ValueError('Unsupported project kind')
    stop_previous(root)
    with (release / 'sovra-output.log').open('wb') as log:
        child = subprocess.Popen(command, cwd=str(release), stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
    active = {'pid': child.pid, 'directory': str(release), 'identity': process_identity(child.pid)}
    (root / 'active.json').write_text(json.dumps(active))
    time.sleep(1)
    code = child.poll()
    print(json.dumps({'directory': str(release), 'pid': child.pid, 'exit_code': code, 'running': code is None, 'kind': kind}))

try:
    deploy(PAYLOAD, pathlib.Path('/proc/device-tree/model').read_text().strip().strip('\x00'), pathlib.Path.home() / '.local' / 'share' / 'sovra' / 'projects')
except (OSError, ValueError, KeyError) as error:
    print(json.dumps({'error': str(error)[:300]}))
