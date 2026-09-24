<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { goto } from "@roxi/routify"
    import { setTimeout } from "worker-timers"

    import { showInExplorer } from "@/functions"
    import { appInfo, assistantVoice, translations, translate } from "@/stores"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"

    import {
        Notification,
        Button,
        Text,
        Tabs,
        Space,
        Alert,
        Input,
        InputWrapper,
        NativeSelect,
        Switch,
        Textarea
    } from "@svelteuidev/core"

    import {
        Check,
        Mix,
        Cube,
        Code,
        Gear,
        QuestionMarkCircled,
        CrossCircled,
        Person
    } from "radix-icons-svelte"

    $: t = (key: string) => translate($translations, key)

    interface VoiceMeta {
        id: string
        name: string
        author: string
        languages: string[]
    }

    interface VoiceConfig {
        voice: VoiceMeta
    }
    
    let availableVoices: VoiceMeta[] = []

    async function selectVoice(voiceId: string) {
        voiceVal = voiceId
        
        // play preview sound
        try {
            await invoke("preview_voice", { voiceId })
        } catch (err) {
            console.error("Failed to preview voice:", err)
        }
    }

    // ### STATE
    interface MicrophoneOption {
        label: string
        value: string
    }

    let availableMicrophones: MicrophoneOption[] = []
    let availableVoskModels: { label: string; value: string }[] = []
    let availableGlinerModels: { label: string; value: string }[] = []
    let settingsSaved = false
    let saveButtonDisabled = false

    // form values (state vars)
    let voiceVal = ""
    let selectedMicrophone = ""
    let selectedWakeWordEngine = ""
    let selectedIntentRecognitionEngine = ""
    let selectedSlotExtractionEngine = ""
    let selectedGlinerModel = ""
    let selectedVoskModel = ""
    let selectedNoiseSuppression = ""
    let selectedVad = ""
    let gainNormalizerEnabled = false
    let apiKeyPicovoice = ""

    // fork: assistant.toml values
    let geminiKeysText = ""
    let paidKey = ""
    let freeOnly = false
    let sttEngine = "whisper"
    let ttsBackend = "sapi"
    let address = "сэр"
    let voiceServer = { installed: false, running: false, gpu: null, stt_engine: null, tts_device: null }
    let assistantError = ""
    let actionMessage = ""
    let update: { current: string; latest: string; available: boolean } | null = null
    let updateBusy = false

    // subscribe to stores
    assistantVoice.subscribe(value => {
        voiceVal = value
    })

    let logFilePath = ""
    appInfo.subscribe(info => {
        logFilePath = info.logFilePath
    })

    // ### FUNCTIONS
    async function saveSettings() {
        saveButtonDisabled = true
        settingsSaved = false

        try {
            await Promise.all([
                invoke("db_write", { key: "assistant_voice", val: voiceVal }),
                invoke("db_write", { key: "selected_microphone", val: selectedMicrophone }),
                invoke("db_write", { key: "selected_wake_word_engine", val: selectedWakeWordEngine }),
                invoke("db_write", { key: "selected_vosk_model", val: selectedVoskModel }),

                invoke("db_write", { key: "noise_suppression", val: selectedNoiseSuppression }),
                invoke("db_write", { key: "gain_normalizer", val: gainNormalizerEnabled.toString() }),

                invoke("assistant_settings_write", {
                    settings: {
                        gemini_keys: geminiKeysText.split(/[\s,;]+/).filter((k) => k.length > 0),
                        stt_engine: sttEngine,
                        tts_backend: ttsBackend,
                        address: address,
                        paid_key: paidKey.trim(),
                        free_only: freeOnly,
                    },
                }),
            ])
            assistantError = ""

            // settings are read at start: restart Jarvis if it is running
            if (await invoke<boolean>("is_jarvis_app_running")) {
                invoke("restart_jarvis_app").catch((err) => console.error("restart failed:", err))
            }

            // update shared store
            assistantVoice.set(voiceVal)
            settingsSaved = true

            // hide alert after 5 seconds
            setTimeout(() => {
                settingsSaved = false
            }, 5000)

            // restart listening with new settings
            // stopListening(() => startListening())
        } catch (err) {
            assistantError = String(err)
            console.error("failed to save settings:", err)
        }

        setTimeout(() => {
            saveButtonDisabled = false
        }, 1000)
    }

    async function openConfigFile() {
        try { await invoke("open_assistant_config") } catch (err) { console.error("open config:", err) }
    }

    async function collectLogs() {
        actionMessage = "Собираю логи…"
        try {
            const path = await invoke<string>("collect_logs")
            actionMessage = `Готово: ${path}. Отправьте этот файл (ключи в нём скрыты).`
            showInExplorer(path)
        } catch (err) {
            actionMessage = `Не удалось собрать логи: ${err}`
            console.error("collect logs:", err)
        }
    }

    async function checkUpdate() {
        updateBusy = true
        actionMessage = ""
        try {
            update = await invoke("check_update")
            if (update && !update.available) actionMessage = `Установлена последняя версия (${update.current}).`
        } catch (err) {
            actionMessage = `Не удалось проверить обновления: ${err}`
        }
        updateBusy = false
    }

    async function installUpdate() {
        updateBusy = true
        actionMessage = "Скачиваю обновление… Джарвис перезапустится сам."
        try {
            await invoke("install_update")
        } catch (err) {
            actionMessage = `Не удалось обновить: ${err}`
            updateBusy = false
        }
    }

    // ### INIT
    onMount(async () => {
        try {
            const a = await invoke<{ gemini_keys: string[]; stt_engine: string; tts_backend: string; address: string; paid_key: string; free_only: boolean }>("assistant_settings_read")
            geminiKeysText = a.gemini_keys.join("\n")
            paidKey = a.paid_key || ""
            freeOnly = !!a.free_only
            sttEngine = a.stt_engine
            ttsBackend = a.tts_backend
            address = a.address || "сэр"
        } catch (err) {
            assistantError = String(err)
            console.error("failed to read assistant.toml:", err)
        }
        try {
            voiceServer = await invoke("voice_server_status")
        } catch (err) {
            console.error("voice server status:", err)
        }

        // load voices
        try {
            const voices = await invoke<VoiceConfig[]>("list_voices")
            availableVoices = voices.map(v => v.voice)
        } catch (err) {
            console.error("Failed to load voices:", err)
            availableVoices = []
        }

        try {
            // load microphones
            const mics = await invoke<string[]>("pv_get_audio_devices")
            availableMicrophones = [
                { label: t('settings-mic-default'), value: "-1" },  // system default
                ...mics.map((name, idx) => ({
                    label: name,
                    value: String(idx)
                }))
            ]

            // load vosk models
            const languageNames: Record<string, string> = {
                us: 'English',
                ru: 'Русский',
                uk: 'Українська',
                de: 'German',
                fr: 'French',
                es: 'Spanish',
                // ..
            };
            const voskModels = await invoke<{ name: string; language: string; size: string }[]>("list_vosk_models")
            availableVoskModels = voskModels.map(m => ({
                label: `${m.name} (${languageNames[m.language] ?? m.language}, ${m.size})`,
                value: m.name
            }))

            // load gliner models
            const glinerModels = await invoke<{ display_name: string; value: string }[]>("list_gliner_models")
            availableGlinerModels = glinerModels.map(m => ({
                label: m.display_name,
                value: m.value,
            }))

            // load settings from db
            const [mic, wakeWord, intentReco, slotEngine, glinerModel, voskModel,
                   noiseSuppression, vad, gainNormalizer,
                   pico] = await Promise.all([
                invoke<string>("db_read", { key: "selected_microphone" }),
                invoke<string>("db_read", { key: "selected_wake_word_engine" }),
                invoke<string>("db_read", { key: "selected_intent_recognition_engine" }),
                invoke<string>("db_read", { key: "selected_slot_extraction_engine" }),
                invoke<string>("db_read", { key: "selected_gliner_model" }),
                invoke<string>("db_read", { key: "selected_vosk_model" }),

                invoke<string>("db_read", { key: "noise_suppression" }),
                invoke<string>("db_read", { key: "vad" }),
                invoke<string>("db_read", { key: "gain_normalizer" }),

                invoke<string>("db_read", { key: "api_key__picovoice" })
            ])

            selectedMicrophone = mic
            selectedWakeWordEngine = wakeWord
            selectedIntentRecognitionEngine = intentReco
            selectedSlotExtractionEngine = slotEngine
            selectedVoskModel = voskModel
            selectedGlinerModel = glinerModel
            selectedNoiseSuppression = noiseSuppression
            selectedVad = vad
            gainNormalizerEnabled = gainNormalizer === "true"
            apiKeyPicovoice = pico
        } catch (err) {
            console.error("failed to load settings:", err)
        }
    })
</script>

<Space h="xl" />

<Notification
    title={t('settings-beta-title')}
    icon={QuestionMarkCircled}
    color="blue"
    withCloseButton={false}
>
    {t('settings-beta-desc')}<br />
    О найденных ошибках пишите на <b>spoke696@gmail.com</b> или в
    <a href="https://github.com/softlyfear/jarvis/issues" target="_blank">Issues на GitHub</a>
    — приложите архив из кнопки «Собрать логи для отправки».
    <Space h="sm" />
    <Button
        color="gray"
        radius="md"
        size="xs"
        uppercase
        on:click={() => showInExplorer(logFilePath)}
    >
        {t('settings-open-logs')}
    </Button>
</Notification>

<Space h="xl" />

{#if settingsSaved}
    <Notification
        title={t('notification-saved')}
        icon={Check}
        color="teal"
        on:close={() => { settingsSaved = false }}
    />
    <Space h="xl" />
{/if}

<Tabs class="form" color="#8AC832" position="left">
    <Tabs.Tab label="Джарвис" icon={Person}>
        <Space h="sm" />
        <InputWrapper label="Ключи Gemini">
            <Text size="sm" color="gray">
                Нейросеть для разговора и сложных просьб. Бесплатный ключ:
                <a href="https://aistudio.google.com/apikey" target="_blank">aistudio.google.com/apikey</a>
                (из России — с VPN). Несколько ключей — каждый с новой строки; модель выбирается автоматически.
            </Text>
            <Space h="xs" />
            <Textarea placeholder="AIza..." variant="filled" minRows={2} autosize bind:value={geminiKeysText} />
        </InputWrapper>

        <Space h="xl" />
        <InputWrapper label="Платные модели (ключ Kilo или OpenRouter)">
            <Text size="sm" color="gray">
                Необязательно. С ключом первой отвечает дешёвая платная модель (DeepSeek V4 Flash, около цента в день),
                без VPN. Ключ: <a href="https://app.kilo.ai" target="_blank">app.kilo.ai</a> или
                <a href="https://openrouter.ai/keys" target="_blank">openrouter.ai/keys</a>.
                Кончились деньги — Джарвис сам переходит на бесплатные модели.
            </Text>
            <Space h="xs" />
            <Textarea placeholder="sk-or-... или ключ Kilo" variant="filled" minRows={1} autosize bind:value={paidKey} />
        </InputWrapper>

        <Space h="md" />
        <InputWrapper label="Только бесплатные модели">
            <Text size="sm" color="gray">
                Не тратить деньги и не ходить в Gemini: отвечают бесплатные модели Kilo без ключа (до 200 запросов в час).
            </Text>
            <Space h="xs" />
            <Switch label={freeOnly ? "Включено" : "Выключено"} bind:checked={freeOnly} />
        </InputWrapper>

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: "Сэр", value: "сэр" },
                { label: "Мисс", value: "мисс" },
                ...(["сэр", "мисс"].includes(address) ? [] : [{ label: address, value: address }])
            ]}
            label="Как Джарвис к вам обращается"
            description="В ответах нейросети и, с голосовым сервером, в коротких откликах («Слушаю, мисс»). Своё слово — в файле настроек, [assistant] address."
            variant="filled"
            bind:value={address}
        />

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: "Whisper — точнее (нужен голосовой сервер)", value: "whisper" },
                { label: "Vosk — встроенный, быстрее и проще", value: "vosk" }
            ]}
            label="Распознавание команд"
            description={voiceServer.installed
                ? (voiceServer.running
                    ? "Голосовой сервер работает. Видеокарта: " + (voiceServer.gpu || "не найдена")
                        + ". Распознавание: " + (voiceServer.stt_engine || "выключено")
                        + ". Голос: " + (voiceServer.tts_device || "выключен") + "."
                    : "Голосовой сервер установлен, запускается вместе с Джарвисом.")
                : "Голосовой сервер не установлен: команды распознаёт Vosk. Установить — галочкой в установщике."}
            variant="filled"
            bind:value={sttEngine}
        />

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: "Голос Джарвиса (нужен голосовой сервер)", value: "http" },
                { label: "Голос Windows", value: "sapi" },
                { label: "Не озвучивать, только уведомление", value: "none" }
            ]}
            label="Голос ответов нейросети"
            description="Короткие отклики с обращением «сэр» звучат записанным голосом Джарвиса, с другим обращением — голосом с сервера."
            variant="filled"
            bind:value={ttsBackend}
        />

        {#if assistantError}
            <Space h="sm" />
            <Alert title="Ошибка настроек" color="red" variant="outline">{assistantError}</Alert>
        {/if}

        <Space h="xl" />
        <div class="tools-row">
            <Button color="gray" radius="md" size="xs" uppercase on:click={openConfigFile}>Файл настроек</Button>
            <Button color="gray" radius="md" size="xs" uppercase on:click={collectLogs}>Собрать логи для отправки</Button>
            {#if update?.available}
                <Button color="green" radius="md" size="xs" uppercase disabled={updateBusy} on:click={installUpdate}>
                    Обновить до {update.latest}
                </Button>
            {:else}
                <Button color="gray" radius="md" size="xs" uppercase disabled={updateBusy} on:click={checkUpdate}>Проверить обновления</Button>
            {/if}
        </div>
        {#if actionMessage}
            <Space h="sm" />
            <Text size="sm" color="gray">{actionMessage}</Text>
        {/if}
    </Tabs.Tab>

    <Tabs.Tab label={t('settings-general')} icon={Gear}>
        <Space h="sm" />
        <div class="voice-select">
            <label>{t('settings-voice')}</label>
            <p class="description">{t('settings-voice-desc')}</p>
            
            <div class="voice-options">
                {#each availableVoices as voice}
                    <button 
                        type="button"
                        class="voice-option"
                        class:selected={voiceVal === voice.id}
                        on:click={() => selectVoice(voice.id)}
                    >
                        <div class="voice-info">
                            <span class="voice-name">{voice.name}</span>
                            {#if voice.author}
                                <span class="voice-author">by {voice.author}</span>
                            {/if}
                        </div>
                        <div class="voice-languages">
                            {#each voice.languages as lang}
                                <img 
                                    src="/media/flags/{lang.toUpperCase()}.png" 
                                    alt={lang} 
                                    width="20" 
                                    title={lang}
                                />
                            {/each}
                        </div>
                    </button>
                {/each}
                
                {#if availableVoices.length === 0}
                    <p class="no-voices">{t('settings-no-voices')}</p>
                {/if}
            </div>
        </div>
    </Tabs.Tab>

    <Tabs.Tab label={t('settings-devices')} icon={Mix}>
        <Space h="sm" />
        <NativeSelect
            data={availableMicrophones}
            label={t('settings-microphone')}
            description={t('settings-microphone-desc')}
            variant="filled"
            bind:value={selectedMicrophone}
        />
    </Tabs.Tab>

    <Tabs.Tab label={t('settings-neural-networks')} icon={Cube}>
        <Space h="sm" />
        <NativeSelect
            data={[
                { label: "Vosk (рекомендуется)", value: "Vosk" },
                { label: "Rustpotter", value: "Rustpotter" }
            ]}
            label={t('settings-wake-word-engine')}
            description={t('settings-wake-word-desc')}
            variant="filled"
            bind:value={selectedWakeWordEngine}
        />

        <Space h="xl" />
        {#key availableVoskModels}
        <NativeSelect
            data={[
                { label: t('settings-auto-detect'), value: "" },
                ...availableVoskModels
            ]}
            label={t('settings-vosk-model')}
            description={t('settings-vosk-model-desc')}
            variant="filled"
            bind:value={selectedVoskModel}
        />
        {/key}

        {#if availableVoskModels.length === 0}
            <Space h="sm" />
            <Alert title={t('settings-models-not-found')} color="orange" variant="outline">
                <Text size="sm" color="gray">
                    {t('settings-models-hint')}
                </Text>
            </Alert>
        {/if}

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: t('settings-disabled'), value: "None" },
                { label: "Nnnoiseless", value: "Nnnoiseless" }
            ]}
            label={t('settings-noise-suppression')}
            description={t('settings-noise-suppression-desc')}
            variant="filled"
            bind:value={selectedNoiseSuppression}
        />

        <Space h="md" />

        <InputWrapper label={t('settings-gain-normalizer')}>
            <Text size="sm" color="gray">
                {t('settings-gain-normalizer-desc')}
            </Text>
            <Space h="xs" />
            <Switch
                label={gainNormalizerEnabled ? t('settings-enabled') : t('settings-disabled')}
                bind:checked={gainNormalizerEnabled}
            />
        </InputWrapper>

    </Tabs.Tab>
</Tabs>

<Space h="xl" />

<Button
    color="lime"
    radius="md"
    size="sm"
    uppercase
    ripple
    fullSize
    on:click={saveSettings}
    disabled={saveButtonDisabled}
>
    {t('settings-save')}
</Button>

<Space h="sm" />

<Button
    color="gray"
    radius="md"
    size="sm"
    uppercase
    fullSize
    on:click={() => $goto("/")}
>
    {t('settings-back')}
</Button>

<HDivider />
<Footer />

<style lang="scss">
.tools-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.voice-select {
    margin-bottom: 1rem;
    
    label {
        font-weight: 600;
        font-size: 0.9rem;
        color: #fff;
        display: block;
        margin-bottom: 0.25rem;
    }
    
    .description {
        font-size: 0.75rem;
        color: rgba(255,255,255,0.5);
        margin: 0 0 0.75rem;
        white-space: pre-line;
    }
}

$voice-item-height: 70px;
$voice-item-gap: 0.5rem;
$voice-max-visible: 3;

.voice-options {
    display: flex;
    flex-direction: column;
    gap: $voice-item-gap;
    max-height: $voice-item-height * $voice-max-visible;
    overflow-y: auto;
    
    &::-webkit-scrollbar {
        width: 6px;
    }
    
    &::-webkit-scrollbar-track {
        background: rgba(255, 255, 255, 0.05);
        border-radius: 3px;
    }
    
    &::-webkit-scrollbar-thumb {
        background: rgba(255, 255, 255, 0.2);
        border-radius: 3px;
        
        &:hover {
            background: rgba(255, 255, 255, 0.3);
        }
    }
}

.voice-option {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem 1rem;
    background: rgba(30, 40, 45, 0.8);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s ease;
    text-align: left;
    width: 100%;
    
    &:hover {
        background: rgba(40, 55, 60, 0.9);
        border-color: rgba(255,255,255,0.2);
    }
    
    &.selected {
        background: rgba(82, 254, 254, 0.1);
        border-color: rgba(82, 254, 254, 0.4);
    }
}

.voice-info {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.15rem;
}

.voice-name {
    font-size: 0.85rem;
    color: #fff;
    font-weight: 500;
}

.voice-author {
    font-size: 0.7rem;
    color: rgba(255,255,255,0.4);
}

.voice-languages {
    display: flex;
    gap: 0.35rem;
    
    img {
        opacity: 0.8;
        border-radius: 2px;
    }
}

.no-voices {
    font-size: 0.8rem;
    color: rgba(255,255,255,0.4);
    font-style: italic;
}
</style>