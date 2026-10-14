"""Exercise grouped settings in Chromium with mocked Tauri; no real credentials/API calls."""
import json
import subprocess
from pathlib import Path

from playwright.sync_api import sync_playwright, expect
from shoot import ROOT, CODEX, FRONTEND, FIXTURES, MOCK_JS, load_ftl, wait_port

OUT = CODEX / 'evidence/settings-2026-10-14'
SHOTS = CODEX / '.tmp/gui/settings-2026-10-14'
LABELS = ['Голос', 'Нейросеть', 'Экран', 'Память', 'Система']
INTERCEPT = """
(() => {
 const original = window.__TAURI_INTERNALS__.invoke;
 window.__SETTINGS_WRITES__ = [];
 window.__FAIL_SAVE__ = false;
 let facts = [{text: 'Люблю игры и читаю длинные-названия-без-пробелов-для-проверки-переноса', added: 1}];
 window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
   if (cmd === 'assistant_settings_write') {
     if (window.__FAIL_SAVE__) throw new Error('Тестовая ошибка сохранения');
     window.__SETTINGS_WRITES__.push({cmd, args});
   }
   if (cmd === 'db_write_many') window.__SETTINGS_WRITES__.push({cmd, args});
   if (cmd === 'user_memory_list') return structuredClone(facts);
   if (cmd === 'user_memory_forget') { facts.splice(args.index, 1); return null; }
   return original(cmd, args);
 };
})();
"""


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    SHOTS.mkdir(parents=True, exist_ok=True)
    vite = subprocess.Popen(['node', str(FRONTEND / 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '1420', '--strictPort'], cwd=FRONTEND, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    results = []
    try:
        wait_port(1420, 60)
        with sync_playwright() as p:
            browser = p.chromium.launch()
            for width in [550, 400]:
                fixtures = dict(FIXTURES, get_translations=load_ftl('ru'))
                fixtures.update(list_vosk_models=[{'name': 'vosk-ru', 'language': 'ru', 'size': '45 MB'}],
                                check_update={'current': '0.2.87', 'latest': '0.2.87', 'available': False})
                fixtures['voice_server_status'] = dict(fixtures['voice_server_status'], stt_engine='gigaam')
                page = browser.new_page(viewport={'width': width, 'height': 800})
                errors = []
                page.on('pageerror', lambda e: errors.append(e.stack or str(e)))
                page.on('console', lambda m: errors.append(m.text) if m.type == 'error' and 'Тестовая ошибка сохранения' not in m.text else None)
                page.add_init_script(MOCK_JS.replace('__FIXTURES__', json.dumps(fixtures)).replace('__STATE__', '"idle"') + INTERCEPT)
                page.goto('http://127.0.0.1:1420/settings')
                expect(page.locator('#settings-heading')).to_have_text('Голос')
                expect(page.locator('select').first).to_have_value('-1')
                for index, label in enumerate(LABELS):
                    nav = page.locator('.settings-nav button').nth(index)
                    nav.focus()
                    page.keyboard.press('Enter')
                    expect(nav).to_have_attribute('aria-pressed', 'true')
                    expect(page.locator('#settings-heading')).to_have_text(label)
                    assert page.locator('.settings-nav button[aria-pressed="true"]').count() == 1
                    assert page.evaluate('document.documentElement.scrollWidth <= window.innerWidth'), label
                    assert page.locator('#header button, .settings-nav button, .save-row button').evaluate_all('(elements) => elements.every(e => { const b = e.getBoundingClientRect(); return b.left >= 0 && b.right <= window.innerWidth; })'), 'buttons must fit the viewport'
                    page.evaluate('window.scrollTo(0, 0)')
                    assert page.locator('.save-row button').evaluate_all('(elements) => elements.every(e => { const b = e.getBoundingClientRect(); return b.top >= 0 && b.bottom <= window.innerHeight; })'),  'save actions must be visible in an 800px viewport'
                    page.wait_for_function("document.querySelector('main > div[aria-hidden=\"true\"]').clientHeight === document.querySelector('.settings-actions').clientHeight + 16")
                    page.screenshot(path=str(SHOTS / f'{width}-{index}.png'), full_page=True)
                    page.screenshot(path=str(SHOTS / f'{width}-{index}-viewport.png'))
                page.locator('.settings-nav button').nth(0).click()
                selects = page.locator('#settings-section select')
                selects.nth(0).select_option('1')
                selects.nth(1).select_option('none')
                selects.nth(2).select_option('мисс')
                page.locator('summary').click()
                selects.nth(3).select_option('Nnnoiseless')
                page.locator('#settings-section input[type=checkbox]').check()
                assert selects.count() == 4, 'one Vosk model must not expose a selector'
                page.screenshot(path=str(SHOTS / f'{width}-advanced.png'), full_page=True)
                page.locator('.settings-nav button').nth(1).click()
                page.get_by_role('combobox', name='Подключение нейросети', exact=True).select_option('kilo')
                page.locator('input[aria-label="Ключ Kilo"]').fill('eyJ-mock-only')
                page.get_by_text('Только бесплатные модели', exact=True).click()
                page.locator('.settings-nav button').nth(2).click()
                page.locator('input[aria-label="Ключ Google AI Studio"]').fill('vision-mock-only')
                # Return to both sections before saving and verify mounted fields retain values.
                page.locator('.settings-nav button').nth(0).click()
                expect(page.locator('#settings-section select').nth(0)).to_have_value('1')
                expect(page.locator('#settings-section select').nth(1)).to_have_value('none')
                page.locator('.settings-nav button').nth(1).click()
                expect(page.locator('input[aria-label="Ключ Kilo"]')).to_have_value('eyJ-mock-only')
                page.get_by_role('combobox', name='Подключение нейросети', exact=True).select_option('polza')
                expect(page.locator('input[aria-label="Ключ Polza AI"]')).to_have_value('sk-test')
                page.get_by_role('combobox', name='Подключение нейросети', exact=True).select_option('kilo')
                page.locator('.settings-nav button').nth(3).click()
                page.get_by_role('button', name='Забыть', exact=True).click()
                expect(page.locator('.memory-fact')).to_have_count(0)
                page.locator('.settings-nav button').nth(4).click()
                page.get_by_role('button', name='Проверить обновления', exact=True).click()
                expect(page.get_by_text('Установлена последняя версия (0.2.87).', exact=True)).to_be_visible()
                page.get_by_role('button', name='Сохранить', exact=True).click()
                page.wait_for_function('window.__TAURI_CALLS__.includes("restart_jarvis_app")')
                writes = page.evaluate('window.__SETTINGS_WRITES__')
                db = dict(next(w['args']['values'] for w in writes if w['cmd'] == 'db_write_many'))
                assert db == {'selected_microphone': '1', 'selected_wake_word_engine': 'Vosk', 'selected_vosk_model': '', 'noise_suppression': 'Nnnoiseless', 'gain_normalizer': 'true'}
                a = next(w['args']['settings'] for w in writes if w['cmd'] == 'assistant_settings_write')
                assert a == {'kilo_key': 'eyJ-mock-only', 'polza_key': 'sk-test', 'gateway': 'kilo', 'free_only': True, 'tts_backend': 'none', 'address': 'мисс', 'vision_key': 'vision-mock-only'}
                page.wait_for_timeout(1200)
                page.evaluate('window.__FAIL_SAVE__ = true')
                before = page.evaluate('window.__TAURI_CALLS__.filter(c => c === "restart_jarvis_app").length')
                page.get_by_role('button', name='Сохранить', exact=True).click()
                expect(page.get_by_text('Error: Тестовая ошибка сохранения', exact=True)).to_be_visible()
                page.locator('.settings-nav button').nth(2).click()
                expect(page.get_by_text('Error: Тестовая ошибка сохранения', exact=True)).to_be_visible()
                assert page.evaluate('window.__TAURI_CALLS__.filter(c => c === "restart_jarvis_app").length') == before
                page.wait_for_function("document.querySelector('main > div[aria-hidden=\"true\"]').clientHeight === document.querySelector('.settings-actions').clientHeight + 16")
                assert page.locator('.save-row button').evaluate_all('(elements) => elements.every(e => { const b = e.getBoundingClientRect(); return b.top >= 0 && b.bottom <= window.innerHeight; })')
                page.screenshot(path=str(SHOTS / f'{width}-save-error.png'), full_page=True)
                page.screenshot(path=str(SHOTS / f'{width}-save-error-viewport.png'))
                assert not errors, errors
                results.append({'width': width, 'checks': ['five sections / keyboard', 'save visible without scrolling', 'no horizontal overflow', 'single model hidden', 'bindings persist', 'both gateway keys preserved', 'all settings saved / restart', 'forget fact', 'check update', 'save error global / no restart'], 'result': 'pass'})
                page.close()
            # Missing / multiple models and unavailable synthesis are distinct states.
            for count in [0, 2]:
                fixtures = dict(FIXTURES, get_translations=load_ftl('ru'))
                fixtures['list_vosk_models'] = [{'name': f'vosk-{i}', 'language': 'ru', 'size': '45 MB'} for i in range(count)]
                fixtures['voice_server_status'] = dict(fixtures['voice_server_status'], tts_error='Тест: GPU недоступна')
                page = browser.new_page(viewport={'width': 550, 'height': 800})
                page.add_init_script(MOCK_JS.replace('__FIXTURES__', json.dumps(fixtures)).replace('__STATE__', '"idle"') + INTERCEPT)
                page.goto('http://127.0.0.1:1420/settings')
                expect(page.get_by_text('Тест: GPU недоступна', exact=True)).to_be_visible()
                page.locator('summary').click()
                expect(page.locator('#settings-section select')).to_have_count(5 if count else 4)
                if count == 0:
                    expect(page.get_by_text(load_ftl('ru')['settings-models-not-found'], exact=True)).to_be_visible()
                else:
                    page.locator('#settings-section select').nth(4).select_option('vosk-1')
                    page.locator('.settings-nav button').nth(2).click()
                    page.locator('.settings-nav button').nth(0).click()
                    page.locator('summary').click()
                    expect(page.locator('#settings-section select').nth(4)).to_have_value('vosk-1')
                page.screenshot(path=str(SHOTS / f'models-{count}.png'), full_page=True)
                results.append({'model_count': count, 'result': 'pass'})
                page.close()
            browser.close()
        report = {'result': 'pass', 'environment': 'Linux / headless Chromium / mocked Tauri', 'checks': results, 'screenshots': str(SHOTS.relative_to(ROOT)), 'limits': 'Native Tauri, Windows hardware, real persistence and API are not exercised.'}
        (OUT / 'gui-checks.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps(report, ensure_ascii=False, indent=2))
    finally:
        vite.terminate()
        vite.wait(10)


if __name__ == '__main__':
    main()
