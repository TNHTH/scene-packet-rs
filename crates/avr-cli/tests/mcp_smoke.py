#!/usr/bin/env python3
"""Linux 合成端到端：真实 FFmpeg、模拟 ASR、全新 MCP stdio 进程。"""
import base64
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import tempfile
import time

repo = Path(__file__).resolve().parents[3]
binary = Path(os.environ.get('CARGO_TARGET_DIR', repo / 'target')) / 'debug/scene-packet'

def call(process, identifier, method, params=None):
    process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': identifier, 'method': method, 'params': params or {}}) + '\n')
    process.stdin.flush()
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if not select.select([process.stdout], [], [], max(0, deadline-time.monotonic()))[0]:
            break
        line = process.stdout.readline()
        if not line:
            raise AssertionError('MCP exited before response')
        response = json.loads(line)
        if response.get('id') == identifier:
            return response
    raise AssertionError('MCP response timed out')

def value(response):
    assert 'error' not in response, response
    assert not response['result'].get('isError'), response
    return json.loads(response['result']['content'][0]['text'])

with tempfile.TemporaryDirectory() as temp:
    root = Path(temp)
    video = root / '中文 视频.mp4'
    subprocess.run(['/usr/bin/ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=c=blue:s=320x240:r=5:d=1', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1', '-shortest', '-c:v', 'mpeg4', str(video)], check=True)
    fake = root / 'whisper-test'
    fake.write_text("#!/usr/bin/env python3\nimport pathlib,sys\nif '--help' in sys.argv: print('synthetic whisper');sys.exit(0)\np=pathlib.Path(sys.argv[sys.argv.index('-of')+1]+'.srt')\np.write_text('1\\n00:00:00,000 --> 00:00:00,500\\nsynthetic speech\\n\\n2\\n00:00:00,950 --> 00:00:01,000\\n如图\\n')\n")
    fake.chmod(0o755)
    model = root / 'model.bin'; model.write_bytes(b'synthetic model')
    data = root / 'data'; data.mkdir()
    (data/'config.toml').write_text('[asr]\nactive_preset="whisper-cpp-cpu"\n[tools]\nffmpeg_path="/usr/bin/ffmpeg"\nffprobe_path="/usr/bin/ffprobe"\nwhisper_cli_path='+json.dumps(str(fake))+'\nwhisper_model_path='+json.dumps(str(model))+'\n')
    prefix = [str(binary), '--data-root', str(data)]
    first = json.loads(subprocess.check_output(prefix+['transcribe', str(video)]))
    renamed = root/'移动后.mp4'; video.rename(renamed)
    process = subprocess.Popen(prefix+['--mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        init = call(process, 1, 'initialize', {'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'synthetic-regression','version':'1'}})
        assert 'result' in init, init
        process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); process.stdin.flush()
        listed=call(process,2,'tools/list')['result']['tools']
        assert {'analyze_media','analyze_video','list_visual_hints','extract_frames','export_packet'} <= {t['name'] for t in listed}
        second=value(call(process,3,'tools/call',{'name':'analyze_media','arguments':{'source':str(renamed)}}))
        assert first['media_key']==second['media_key'] and second['title']==renamed.name
        hints=value(call(process,4,'tools/call',{'name':'list_visual_hints','arguments':{'media_key':second['media_key']}}))
        times=sorted({h['time'] for h in hints})
        assert times[-1] <= .8, times
        response=call(process,5,'tools/call',{'name':'extract_frames','arguments':{'media_key':second['media_key'],'times':times,'delivery':'both'}})
        frames=value(response)
        images=[c for c in response['result']['content'] if c['type']=='image']
        assert len(images)==len(frames)==2
        assert all(base64.b64decode(c['data']).startswith(b'\xff\xd8') for c in images)
        assert all(f['width']==320 and f['height']==240 for f in frames)
        packet=value(call(process,6,'tools/call',{'name':'export_packet','arguments':{'source':str(renamed),'output_dir':str(root/'packet'),'max_frames':2}}))
        assert packet['frame_count']==2
        subprocess.run(['python3',str(repo/'tools/validate-local-video.py'),str(root/'packet'),'--source',str(renamed)],check=True)
        failure=call(process,7,'tools/call',{'name':'extract_frames','arguments':{'media_key':'../escape','times':[0]}})
        assert 'error' in failure or failure.get('result',{}).get('isError'), failure
        print(json.dumps({'mcp':'PASS','tools':len(listed),'tail':times[-1],'image_blocks':len(images),'moved_source_cache':True}))
    finally:
        process.stdin.close()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired: process.kill(); process.wait()
