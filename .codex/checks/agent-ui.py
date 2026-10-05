import importlib.util, json, subprocess, os
from pathlib import Path
from playwright.sync_api import sync_playwright
root=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('shoot',root/'.codex/skills/gui-check/scripts/shoot.py')
shoot=importlib.util.module_from_spec(spec);spec.loader.exec_module(shoot)
out=root/'.codex/.tmp/gui/openclaw';out.mkdir(parents=True,exist_ok=True)
vite=subprocess.Popen(['node','node_modules/vite/bin/vite.js','--host','127.0.0.1','--port','1420','--strictPort'],cwd=root/'frontend',stdout=subprocess.DEVNULL,stderr=subprocess.STDOUT)
try:
 shoot.wait_port(1420,30)
 with sync_playwright() as p:
  browser=p.chromium.launch(headless=True)
  for count in (0,1,2):
   fixtures=dict(shoot.FIXTURES,get_translations=shoot.load_ftl('ru'),agent_connection_status={'connected':True,'message':'Gateway подключён, агент найден.'})
   fixtures['list_vosk_models']=[{'name':f'vosk-model-ru-{i}','language':'ru','size':'50 MB'} for i in range(count)]
   managed={'backend':'openclaw','fallback_backend':'direct','mcp_enabled':True,'openclaw':{'base_url':'http://127.0.0.1:18790','api_key':'managed-private-test-token','agent':'jarvis','model':'','vision_model':'','connect_timeout_secs':3,'request_timeout_secs':60,'task_timeout_secs':300,'max_tool_rounds':32,'streaming':False}}
   fixtures['agent_setup']=managed
   fixtures['agent_models']=['jarvis-provider-0/test-model','jarvis-provider-0/second-model']
   page=browser.new_page(viewport={'width':550,'height':800});errors=[]
   page.on('pageerror',lambda e:errors.append(str(e)))
   page.add_init_script(shoot.MOCK_JS.replace('__FIXTURES__',json.dumps(fixtures)).replace('__STATE__','"idle"').replace('calls.push(cmd);','calls.push(cmd); if (cmd === "assistant_settings_write") window.savedSettings = structuredClone(args.settings);'))
   page.goto('http://127.0.0.1:1420/settings');page.wait_for_load_state('networkidle');page.wait_for_timeout(300)
   if count==1:
    page.get_by_role('combobox').first.select_option('openclaw')
    password=page.locator('input[type="password"]');assert password.count()==1
    password.fill('private-test-token')
    page.get_by_role('button',name='Проверить подключение',exact=True).click()
    page.get_by_text('Gateway подключён, агент найден.',exact=True).wait_for()
    page.screenshot(path=str(out/'settings-openclaw.png'),full_page=True)
    page.get_by_role('button',name='Сохранить',exact=True).click();page.wait_for_timeout(100)
    saved=page.evaluate('window.savedSettings');assert saved['agent']['backend']=='openclaw';assert saved['agent']['openclaw']['api_key']=='private-test-token'
    page.get_by_role('button',name='Настроить OpenClaw',exact=True).click()
    page.get_by_text('OpenClaw настроен. Нажмите «Сохранить», чтобы включить его в Джарвисе.',exact=True).wait_for()
    assert password.input_value()=='managed-private-test-token'
    page.get_by_role('combobox',name='Модели профиля',exact=True).select_option('jarvis-provider-0/second-model')
    page.get_by_role('button',name='Настройки OpenClaw в браузере',exact=True).click()
    page.get_by_role('button',name='Открыть профиль',exact=True).click()
    page.screenshot(path=str(out/'settings-managed-openclaw.png'),full_page=True)
    page.get_by_role('button',name='Сохранить',exact=True).click();page.wait_for_timeout(100)
    saved=page.evaluate('window.savedSettings');assert saved['agent']['mcp_enabled'];assert saved['agent']['openclaw']['model']=='jarvis-provider-0/second-model'
    calls=page.evaluate('window.__TAURI_CALLS__');assert 'agent_setup' in calls and 'agent_open_dashboard' in calls and 'agent_open_profile' in calls
   page.get_by_role('tab',name='Нейросети',exact=True).click()
   page.wait_for_timeout(100)
   model=page.get_by_text('Модель распознавания речи (Vosk)',exact=True)
   assert model.count()==(1 if count>1 else 0),f'model chooser count={count}'
   assert page.get_by_role('combobox').first.locator('option').count()==2
   if count==0:assert page.get_by_text('Модели не найдены',exact=False).count()>0
   page.screenshot(path=str(out/f'networks-{count}-models.png'),full_page=True)
   assert not errors,errors
   print(f'PASS {count} Vosk models, chooser visibility, engine alternatives'+(', OpenClaw token/save/check' if count==1 else ''))
   page.close()
  browser.close()
finally:vite.terminate();vite.wait(10)
