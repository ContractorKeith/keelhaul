# Deliberately small independent v3 peer for Rust client contract tests.
import struct, sys
r, w = sys.stdin.buffer, sys.stdout.buffer
mode = sys.argv[1]
def log(value):
    if len(sys.argv) > 2:
        with open(sys.argv[2], 'a') as out: out.write(str(value)+'\n')
log('spawn')
def u32(n): return struct.pack('>I', n)
def string(b): return u32(len(b)) + b
def packet(t, b):
    w.write(u32(len(b)+1) + bytes([t]) + b)
    w.flush()
def read():
    header = r.read(4)
    if not header: sys.exit(0)
    n = struct.unpack('>I', header)[0]
    b = r.read(n)
    assert len(b) == n
    return b[0], b[1:]
def status(i, code=0): packet(101, i + u32(code) + string(b'fixture diagnostic') + string(b''))
assert read() == (1,u32(3))
if mode == 'oversize':
    w.write(u32(262145)); w.flush(); sys.exit(0)
if mode == 'truncated':
    w.write(u32(10)+b'\x02'); w.flush(); sys.exit(0)
packet(2,u32(3)+string(b'posix-rename@openssh.com')+string(b'1'))
pending = []
while True:
    t,b = read()
    i,b = b[:4],b[4:]
    log(t)
    if mode.startswith('backend-'):
        if t in (7,17) and b == string(b'/repo'):
            packet(105,i+u32(5)+struct.pack('>Q',0)+u32(0o040755)); continue
        if mode == 'backend-missing':
            if t == 11 and b in (string(b'/repo'),string(b'/repo/')): packet(102,i+string(b'h'))
            elif t == 12: status(i,1)
            elif t == 4: status(i)
            else: status(i,2)
            continue
        if t in (7,17):
            packet(105,i+u32(1)+struct.pack('>Q',131072)); continue
        if t == 3:
            packet(102,i+string(b'h')); continue
    if mode == 'malformed':
        packet(102,i+u32(999)); continue
    if mode == 'wrongid':
        status(u32(999)); continue
    if mode == 'missing':
        status(i,2); continue
    if t == 3:
        assert b == string(b'/file')+u32(1)+u32(0)
        packet(102,i+string(b'h'))
    elif t == 8:
        assert b == string(b'h')
        packet(105,i+u32(1)+struct.pack('>Q',131072))
    elif t == 5:
        assert b[:5] == string(b'h')
        offset,count = struct.unpack('>QI',b[5:])
        assert count <= 32768
        data = bytes((offset+j)%251 for j in range(min(count,777) if mode == 'short' else count))
        pending.append((i,data))
        if len(pending) == 4 or mode == 'short' and offset%32768 != 0:
            for req,data in reversed(pending):
                if mode == 'read-error': status(req,3)
                else: packet(103,req+string(data))
            pending.clear()
    elif t == 6:
        assert b[:5] == string(b'h')
        offset = struct.unpack('>Q',b[5:13])[0]
        size = struct.unpack('>I',b[13:17])[0]
        assert size == 32768 and b[17:] == b'x'*size and offset < 131072
        pending.append(i)
        if len(pending) == 4:
            for req in reversed(pending): status(req,3 if mode == 'write-error' else 0)
            pending.clear()
    elif t == 4:
        assert b == string(b'h')
        status(i)
    elif t == 200:
        assert b == string(b'posix-rename@openssh.com')+string(b'/a')+string(b'/b')
        status(i)
    elif t == 18:
        assert b == string(b'/a')+string(b'/b')
        status(i)
    else: raise AssertionError(t)
