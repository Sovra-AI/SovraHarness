# Fixed, read-only probe. Sent to Python over an already authenticated SSH session.
import json
import shutil


def read(path):
    try:
        with open(path, 'r') as source:
            return source.read(4096).strip().strip('\x00')
    except (OSError, ValueError):
        return None


def number(path, divisor=1):
    try:
        return float(read(path).split()[0]) / divisor
    except (AttributeError, ValueError, IndexError, TypeError):
        return None


memory = {}
for line in (read('/proc/meminfo') or '').splitlines():
    fields = line.split()
    if len(fields) >= 2 and fields[0] in ('MemTotal:', 'MemAvailable:'):
        memory[fields[0][:-1]] = int(fields[1])
try:
    disk = shutil.disk_usage('/')
    free_bytes = disk.free
except OSError:
    free_bytes = None
print(json.dumps({
    'model': read('/proc/device-tree/model'),
    'uptime_seconds': number('/proc/uptime'),
    'temperature_c': number('/sys/class/thermal/thermal_zone0/temp', 1000),
    'memory_available_kib': memory.get('MemAvailable'),
    'disk_free_bytes': free_bytes
}))
