#!/usr/bin/env python3
"""Render the checked-out diagram pair inside the existing hosted Web job."""
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import tempfile
import time
import uuid
import zlib

IMAGE = 'plantuml/plantuml@sha256:9b5071ab56a4192aac458ab4b0d42261c22c8109695894ae5be6013c3be6cac1'
SOURCES = ('component_architecture', 'runtime_deployment')
LABEL = 'org.lagrange.r5.architecture-render'
DOCKER = ['docker', '--host', 'unix:///var/run/docker.sock']


class RenderError(Exception):
    pass


def require(condition, code):
    if not condition:
        raise RenderError(code)


def file_info(path):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, 'RENDER_FILE_IDENTITY')
    raw = path.read_bytes()
    return {'name': path.name, 'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}


def png_info(path):
    info = file_info(path)
    raw = path.read_bytes()
    require(raw[:8] == b'\x89PNG\r\n\x1a\n', 'RENDER_PNG_MAGIC')
    offset, chunks, dimensions = 8, [], None
    while offset < len(raw):
        require(offset + 12 <= len(raw), 'RENDER_PNG_TRUNCATED')
        length = struct.unpack('>I', raw[offset:offset + 4])[0]
        end = offset + 12 + length
        require(end <= len(raw), 'RENDER_PNG_TRUNCATED')
        kind = raw[offset + 4:offset + 8]
        body = raw[offset + 8:offset + 8 + length]
        crc = struct.unpack('>I', raw[offset + 8 + length:end])[0]
        require(zlib.crc32(kind + body) & 0xffffffff == crc, 'RENDER_PNG_CRC')
        if not chunks:
            require(kind == b'IHDR' and length == 13, 'RENDER_PNG_IHDR')
            dimensions = struct.unpack('>II', body[:8])
            require(all(0 < x <= 100000 for x in dimensions), 'RENDER_PNG_DIMENSIONS')
        if kind in (b'tEXt', b'zTXt', b'iTXt'):
            require(not body.split(b'\0', 1)[0].lower().startswith(b'plantuml'),
                    'RENDER_PNG_SOURCE_METADATA')
        chunks.append(kind)
        offset = end
        if kind == b'IEND':
            require(length == 0 and end == len(raw), 'RENDER_PNG_TRAILING')
            break
    require(chunks[-1:] == [b'IEND'] and b'IDAT' in chunks, 'RENDER_PNG_INCOMPLETE')
    return dict(info, width=dimensions[0], height=dimensions[1])


def main():
    require(os.environ.get('GITHUB_ACTIONS') == 'true', 'RENDER_HOSTED_ONLY')
    candidate = os.environ.get('GITHUB_SHA', '')
    require(re.fullmatch('[0-9a-f]{40}', candidate), 'RENDER_CANDIDATE')
    end = time.monotonic() + 240

    def run(args, *, maximum=120, cleanup=False):
        remaining = end - time.monotonic() - (0 if cleanup else 20)
        require(remaining > 0, 'RENDER_DEADLINE')
        result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=min(maximum, remaining), check=False)
        require(result.returncode == 0, 'RENDER_COMMAND_FAILED')
        return result.stdout

    require(run(['git', 'rev-parse', 'HEAD']).decode().strip() == candidate, 'RENDER_CHECKOUT')
    root = Path(os.environ['RUNNER_TEMP']).resolve()
    require(root.is_dir(), 'RENDER_TEMP')
    output = root / 'r5-architecture-output'
    output.mkdir(mode=0o700)
    private = Path(tempfile.mkdtemp(prefix='r5-architecture-', dir=root))
    nonce = uuid.uuid4().hex
    name = 'r5-architecture-' + nonce
    input_dir = private / 'input'
    input_dir.mkdir(mode=0o700)
    source_info = []
    cid, image_id, create_sent, cleanup_proven = None, None, False, False
    error, exit_code, pngs, phase = None, None, [], 'SOURCE_COPY'
    observed_limits = None
    try:
        for stem in SOURCES:
            source = Path('docs/diagrams') / (stem + '.puml')
            source_info.append(file_info(source))
            shutil.copyfile(source, input_dir / source.name)
        phase = 'IMAGE_PULL'
        run(DOCKER + ['pull', IMAGE], maximum=120)
        phase = 'IMAGE_INSPECT'
        image = json.loads(run(DOCKER + ['image', 'inspect', IMAGE]))
        require(len(image) == 1, 'RENDER_IMAGE_IDENTITY')
        image_id = image[0]['Id']
        require(re.fullmatch('sha256:[0-9a-f]{64}', image_id), 'RENDER_IMAGE_IDENTITY')
        require(IMAGE in image[0].get('RepoDigests', []), 'RENDER_IMAGE_DIGEST')
        args = DOCKER + ['create', '--name', name, '--label', LABEL + '=' + nonce,
                         '--network', 'none', '--read-only', '--user', f'{os.getuid()}:{os.getgid()}',
                         '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                         '--memory', '768m', '--memory-swap', '768m', '--cpus', '1',
                         '--cpu-shares', '48', '--workdir', '/data', '--restart', 'no',
                         '--tmpfs', '/tmp:rw,nosuid,nodev,size=67108864',
                         '--env', 'JAVA_TOOL_OPTIONS=-Xmx512m',
                         '--mount', f'type=bind,src={input_dir},dst=/data',
                         IMAGE, '-charset', 'UTF-8', '-tpng', '-nometadata', '-failfast2',
                         *('/data/' + stem + '.puml' for stem in SOURCES)]
        phase = 'CONTAINER_CREATE'
        create_sent = True
        cid = run(args, maximum=20).decode().strip()
        require(re.fullmatch('[0-9a-f]{64}', cid), 'RENDER_CONTAINER_IDENTITY')
        phase = 'RENDER'
        run(DOCKER + ['start', '--attach', cid], maximum=180)
        phase = 'RESULT_INSPECT'
        state = json.loads(run(DOCKER + ['inspect', cid], maximum=10))[0]
        require(state['Id'] == cid and state['Image'] == image_id and
                state['Name'] == '/' + name and state['Config']['Labels'].get(LABEL) == nonce,
                'RENDER_CONTAINER_IDENTITY')
        config = state['HostConfig']
        binds = [m for m in state['Mounts'] if m['Type'] == 'bind']
        require(config['NetworkMode'] == 'none' and config['ReadonlyRootfs'] and
                not config['Privileged'] and config['CapDrop'] == ['ALL'] and
                'no-new-privileges' in config['SecurityOpt'] and
                state['Config']['User'] == f'{os.getuid()}:{os.getgid()}' and
                config['Memory'] == config['MemorySwap'] == 805306368 and
                config['NanoCpus'] == 1000000000 and config['CpuShares'] == 48 and
                len(binds) == 1 and binds[0]['Source'] == str(input_dir) and
                binds[0]['Destination'] == '/data' and binds[0]['RW'] and
                all(m['Destination'] in ('/data', '/tmp') for m in state['Mounts']),
                'RENDER_CONTAINER_CONFIG')
        observed_limits = {'memory_bytes': config['Memory'], 'memory_and_swap_bytes': config['MemorySwap'],
                           'nano_cpus': config['NanoCpus'], 'cpu_shares': config['CpuShares'],
                           'network': config['NetworkMode'], 'read_only_root': config['ReadonlyRootfs']}
        require(not state['State']['Running'] and not state['State'].get('OOMKilled') and
                state['State']['ExitCode'] == 0, 'RENDER_CONTAINER_FAILED')
        exit_code = state['State']['ExitCode']
        require(source_info == [file_info(Path('docs/diagrams') / (stem + '.puml')) for stem in SOURCES],
                'RENDER_SOURCE_CHANGED')
        pngs = [png_info(input_dir / (stem + '.png')) for stem in SOURCES]
    except Exception as exc:
        error = exc.code if hasattr(exc, 'code') else (
            str(exc) if type(exc) is RenderError else type(exc).__name__)
    finally:
        try:
            if create_sent:
                found = run(DOCKER + ['ps', '--all', '--no-trunc', '--filter', 'label=' + LABEL + '=' + nonce,
                                      '--format', '{{.ID}}'], maximum=10, cleanup=True).decode().splitlines()
                require(len(found) <= 1, 'RENDER_CLEANUP_IDENTITY')
                for owned_id in found:
                    require(re.fullmatch('[0-9a-f]{64}', owned_id), 'RENDER_CLEANUP_IDENTITY')
                    item = json.loads(run(DOCKER + ['inspect', owned_id], maximum=10, cleanup=True))[0]
                    require(item['Id'] == owned_id and item['Image'] == image_id and
                            item['Name'] == '/' + name and item['Config']['Labels'].get(LABEL) == nonce,
                            'RENDER_CLEANUP_IDENTITY')
                    run(DOCKER + ['rm', '--force', owned_id], maximum=10, cleanup=True)
                require(not run(DOCKER + ['ps', '--all', '--no-trunc', '--filter',
                    'label=' + LABEL + '=' + nonce, '--format', '{{.ID}}'], maximum=10, cleanup=True).strip(),
                    'RENDER_CLEANUP_UNPROVEN')
            cleanup_proven = True
        except Exception:
            error = 'RENDER_CLEANUP_UNPROVEN'
        if error is None and cleanup_proven:
            for stem in SOURCES:
                shutil.copyfile(input_dir / (stem + '.png'), output / (stem + '.png'))
        report = dict(candidate=candidate, image_reference=IMAGE, actual_image_id=image_id,
                      actual_container_id=cid, exit_code=exit_code, cleanup_proven=cleanup_proven,
                      source2=source_info, png2=pngs, status='PASS' if error is None else 'FAIL',
                      error_code=error, phase=phase, requested_network='none',
                      observed_limits=observed_limits,
                      source_metadata_embedded=False if len(pngs) == 2 else 'NOT_VERIFIED')
        with (output / 'render-evidence.json').open('x') as stream:
            json.dump(report, stream, indent=2)
            stream.write('\n')
    require(error is None and cleanup_proven and len(pngs) == 2, error or 'RENDER_FAILED')
    print('R5_ARCHITECTURE_RENDER_PASS')


if __name__ == '__main__':
    try:
        main()
    except (RenderError, subprocess.SubprocessError, OSError):
        raise SystemExit('R5_ARCHITECTURE_RENDER_FAILED')
