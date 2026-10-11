#!/usr/bin/env python3
"""Run the real Rust wizard against deterministic dialog/provider fixtures."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="gardengate-setup-test-") as temp:
    root = Path(temp)
    stubs = root / "bin"
    stubs.mkdir()
    dialog_stub = '''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
root=Path(os.environ['FIXTURE'])
queue=root/'answers.json'
answers=json.loads(queue.read_text())
if not answers: raise SystemExit(1)
expected,value=answers.pop(0)
queue.write_text(json.dumps(answers))
assert expected in sys.argv, (expected,sys.argv)
if value is None: raise SystemExit(1)
print(value)
'''
    rclone_stub = '''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
args=sys.argv[1:]
assert all(' secret ' not in a and '654321' not in a for a in args)
config=Path(args[args.index('--config')+1])
if 'config' in args:
    def question(name,state,**kw):
        print(json.dumps({'State':state,'Option':dict(Name=name,Required=True,**kw),'Error':''}))
    if 'create' in args: question('service','service')
    else:
        state=os.environ['RCLONE_STATE']; answer=os.environ['RCLONE_RESULT']
        if state=='service':
            assert answer=='drive'; question('apple_id','account')
        elif state=='account':
            assert answer=='fixture@example.invalid'; question('password','password',IsPassword=True)
        elif state=='password':
            assert answer==' secret '; question('config_fs_advanced','advanced')
        elif state=='advanced':
            assert answer=='false'; question('config_2fa','code')
        elif state=='code':
            assert answer=='654321'
            config.write_text('RCLONE_ENCRYPT_V0:\nconnected-fixture\n')
            print(json.dumps({'State':'','Option':None,'Error':''}))
        else: raise AssertionError(state)
elif 'lsjson' in args:
    print(json.dumps([{'Name':'Omarchy Inbox','IsDir':True}] if 'icloud:' in args else []))
elif 'size' in args: print(json.dumps({'count':1,'bytes':5}))
else: raise AssertionError(args)
'''
    # Preserve literal newline escapes in the generated fixture's Python string.
    rclone_stub = rclone_stub.replace("'RCLONE_ENCRYPT_V0:\nconnected-fixture\n'", "'RCLONE_ENCRYPT_V0:\\nconnected-fixture\\n'")
    for name, code in {"kdialog": dialog_stub, "rclone": rclone_stub}.items():
        path = stubs / name
        path.write_text(code)
        path.chmod(0o755)

    for cancel in (True, False):
        home = root / ("cancel" if cancel else "complete")
        config = home / ".config/gardengate"
        config.mkdir(parents=True)
        connection = config / "rclone.conf"
        original = b"RCLONE_ENCRYPT_V0:\nprevious-fixture\n"
        connection.write_bytes(original)
        answers = [["--menu", "connect"], ["--yesno", ""],
                   ["--inputbox", "fixture@example.invalid"], ["--password", None if cancel else " secret "]]
        if not cancel:
            answers += [["--inputbox", "654321"], ["--msgbox", ""],
                        ["--menu", "0"], ["--menu", "use"],
                        ["--inputbox", str(home / "iCloud Drive/Omarchy Inbox")],
                        ["--msgbox", ""], ["--menu", None]]
        (root / "answers.json").write_text(json.dumps(answers))
        env = {k:v for k,v in os.environ.items() if k not in (
            "XDG_CONFIG_HOME", "XDG_STATE_HOME", "HYPRLAND_INSTANCE_SIGNATURE")}
        env.update(HOME=str(home), FIXTURE=str(root), PATH=f"{stubs}:{os.environ['PATH']}")
        result = subprocess.run([str(binary), "manage"], env=env, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stderr
        assert json.loads((root / "answers.json").read_text()) == [], result.stderr
        assert not (config / "setup.rclone.conf").exists()
        if cancel:
            assert connection.read_bytes() == original
            assert not (config / "profile.json").exists()
        else:
            assert b"connected-fixture" in connection.read_bytes()
            profile = json.loads((config / "profile.json").read_text())
            assert profile['remote_folder'] == 'Omarchy Inbox'
            assert profile['approved'] is False
            assert Path(profile['local']).is_dir()
    print('PASS: graphical onboarding, password whitespace, cancellation/connection preservation, folder selection and no download approval')
