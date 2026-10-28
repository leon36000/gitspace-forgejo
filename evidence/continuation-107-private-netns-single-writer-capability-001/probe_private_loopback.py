#!/usr/bin/env python3
"""Check whether an unprivileged Bubblewrap network namespace hides loopback."""
from __future__ import annotations
import errno, json, socket, subprocess, sys
INNER = r'''import socket, socketserver, sys, threading
port=int(sys.argv[1])
class Handler(socketserver.BaseRequestHandler):
 def handle(self):
  data=self.request.recv(64)
  self.request.sendall(b'private-netns-ok:'+data)
server=socketserver.TCPServer(('127.0.0.1',port),Handler)
thread=threading.Thread(target=server.serve_forever,daemon=True); thread.start()
print('ready',flush=True)
if sys.stdin.readline().strip() != 'GO': raise SystemExit(4)
with socket.create_connection(('127.0.0.1',port),timeout=3) as client:
 client.sendall(b'probe')
 result=client.recv(64).decode()
print('inside_result='+result,flush=True)
server.shutdown(); server.server_close(); thread.join()
'''

def main() -> int:
    with socket.socket() as reserve:
        reserve.bind(('127.0.0.1',0))
        port=reserve.getsockname()[1]
    command=['/usr/bin/bwrap','--unshare-all','--unshare-user','--disable-userns','--assert-userns-disabled','--die-with-parent','--new-session','--unshare-pid','--dir','/etc','--ro-bind','/etc/alternatives','/etc/alternatives','--ro-bind','/usr','/usr','--symlink','usr/lib','/lib','--symlink','usr/bin','/bin','--tmpfs','/tmp','--proc','/proc','--dev','/dev','--','/usr/bin/python3','-c',INNER,str(port)]
    proc=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    ready=proc.stdout.readline().strip()
    host_outcome='not_attempted'
    if ready=='ready':
        try:
            with socket.create_connection(('127.0.0.1',port),timeout=0.4) as client:
                client.sendall(b'host')
                host_outcome='unexpectedly_connected:'+client.recv(64).decode(errors='replace')
        except OSError as exc:
            host_outcome=f'blocked:{type(exc).__name__}:{exc.errno}'
        proc.stdin.write('GO\n'); proc.stdin.flush(); proc.stdin.close()
    stdout_tail=proc.stdout.read()
    stderr=proc.stderr.read()
    returncode=proc.wait(timeout=5)
    result={'bwrap_version':subprocess.check_output(['/usr/bin/bwrap','--version'],text=True).strip(),'uid':os_uid(),'namespace_port':port,'inner_ready':ready,'inner_stdout':stdout_tail.strip(),'outer_host_connection':host_outcome,'returncode':returncode,'stderr':stderr.strip(),'expected_outer_errno':errno.ECONNREFUSED}
    print(json.dumps(result,sort_keys=True))
    return 0 if ready=='ready' and 'inside_result=private-netns-ok:probe' in stdout_tail and host_outcome==f'blocked:ConnectionRefusedError:{errno.ECONNREFUSED}' and returncode==0 and not stderr else 1

def os_uid() -> int:
    import os
    return os.getuid()

if __name__=='__main__':
    raise SystemExit(main())
