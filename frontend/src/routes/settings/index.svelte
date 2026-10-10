<script lang="ts">
    import { onMount, onDestroy } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { goto } from "@roxi/routify"
    import { setTimeout, clearTimeout } from "worker-timers"

    import { showInExplorer } from "@/functions"
    import { appInfo, translations, translate, updateStatus, startUpdate } from "@/stores"
    import UpdateProgress from "@/components/elements/UpdateProgress.svelte"

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
        Switch
    } from "@svelteuidev/core"

    import {
        Check,
        Mix,
        Cube,
        Code,
        Gear,
        QuestionMarkCircled,
        CrossCircled
    } from "radix-icons-svelte"

    $: t = (key: string) => translate($translations, key)

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
    let selectedMicrophone = ""
    let selectedIntentRecognitionEngine = ""
    let selectedSlotExtractionEngine = ""
    let selectedGlinerModel = ""
    let selectedVoskModel = ""
    let selectedNoiseSuppression = ""
    let selectedVad = ""
    let gainNormalizerEnabled = false
    let apiKeyPicovoice = ""

    // fork: assistant.toml values
    let kiloKey = ""
    let polzaKey = ""
    // "kilo" | "polza": asked first, the other one is the fallback
    let gateway = "kilo"
    let freeOnly = false
    // Google AI Studio key: looking at the screen
    let visionKey = ""

    // the field shows the key of the chosen gateway
    $: gatewayKey = gateway === "polza" ? polzaKey : kiloKey
    function setGatewayKey(value: string) {
        const key = value.replace(/\s+/g, "")
        // a pasted key tells its gateway: Kilo keys are JWTs ("eyJ…")
        if (key.startsWith("eyJ")) gateway = "kilo"
        else if (key.startsWith("sk-")) gateway = "polza"
        if (gateway === "polza") polzaKey = value
        else kiloKey = value
    }
    function onKeyInput(e: Event) {
        setGatewayKey((e.target as HTMLInputElement).value)
    }
    let ttsBackend = "http"
    let address = "сэр"
    let voiceServer: { installed: boolean; running: boolean; ready: boolean; stt_available: boolean; gpu: string | null; stt_engine: string | null; tts_device: string | null; tts_error: string | null } = { installed: false, running: false, ready: false, stt_available: false, gpu: null, stt_engine: null, tts_device: null, tts_error: null }
    let statusTimer: number | undefined
    let disposed = false
    async function refreshVoiceStatus() {
        try { voiceServer = await invoke("voice_server_status") }
        catch (err) { console.error("voice server status:", err) }
        if (!disposed) statusTimer = setTimeout(refreshVoiceStatus, 5000)
    }
    onDestroy(() => {
        disposed = true
        if (statusTimer !== undefined) clearTimeout(statusTimer)
    })
    let assistantError = ""
    let actionMessage = ""
    let update: { current: string; latest: string; available: boolean } | null = null
    let updateBusy = false

    // subscribe to stores
    let logFilePath = ""
    const unsubscribeInfo = appInfo.subscribe(info => {
        logFilePath = info.logFilePath
    })
    onDestroy(unsubscribeInfo)

    // ### FUNCTIONS
    async function saveSettings() {
        saveButtonDisabled = true
        settingsSaved = false

        try {
            await Promise.all([
                invoke("db_write_many", { values: [
                    ["selected_microphone", selectedMicrophone],
                    // the only wake-word engine the command pipeline works with
                    ["selected_wake_word_engine", "Vosk"],
                    ["selected_vosk_model", selectedVoskModel],
                    ["noise_suppression", selectedNoiseSuppression],
                    ["gain_normalizer", gainNormalizerEnabled.toString()],
                ] }),

                invoke("assistant_settings_write", {
                    settings: {
                        kilo_key: kiloKey.replace(/\s+/g, ""),
                        polza_key: polzaKey.replace(/\s+/g, ""),
                        gateway: gateway,
                        free_only: freeOnly,
                        tts_backend: ttsBackend,
                        address: address,
                        vision_key: visionKey.replace(/\s+/g, ""),
                    },
                }),
            ])
            assistantError = ""

            // settings are read at start: restart Jarvis if it is running
            if (await invoke<boolean>("is_jarvis_app_running")) {
                await invoke("restart_jarvis_app")
            }

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
        actionMessage = ""
        try {
            await startUpdate()
        } catch (err) {
            actionMessage = `Не удалось обновить: ${err}`
        }
    }

    $: updating = $updateStatus.phase === "downloading" || $updateStatus.phase === "starting"

    // facts about the user the LLM keeps (user-memory.json)
    let memoryFacts: { text: string; added: number }[] = []
    async function loadMemory() {
        try { memoryFacts = await invoke("user_memory_list") }
        catch (err) { console.error("failed to read the memory:", err) }
    }
    async function forgetFact(index: number) {
        try { await invoke("user_memory_forget", { index }) }
        catch (err) { assistantError = String(err) }
        await loadMemory()
    }

    // ### INIT
    onMount(async () => {
        try {
            const a = await invoke<{ kilo_key: string; polza_key: string; gateway: string; free_only: boolean; tts_backend: string; address: string; vision_key: string }>("assistant_settings_read")
            kiloKey = a.kilo_key || ""
            polzaKey = a.polza_key || ""
            gateway = a.gateway === "polza" ? "polza" : "kilo"
            freeOnly = !!a.free_only
            ttsBackend = a.tts_backend
            address = a.address || "сэр"
            visionKey = a.vision_key || ""
        } catch (err) {
            assistantError = String(err)
            console.error("failed to read assistant.toml:", err)
        }
        refreshVoiceStatus()
        loadMemory()

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

            if (availableVoskModels.length <= 1) selectedVoskModel = ""

            // load gliner models
            const glinerModels = await invoke<{ display_name: string; value: string }[]>("list_gliner_models")
            availableGlinerModels = glinerModels.map(m => ({
                label: m.display_name,
                value: m.value,
            }))

            // load settings from db
            const [mic, intentReco, slotEngine, glinerModel, voskModel,
                   noiseSuppression, vad, gainNormalizer,
                   pico] = await Promise.all([
                invoke<string>("db_read", { key: "selected_microphone" }),
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
            selectedIntentRecognitionEngine = intentReco
            selectedSlotExtractionEngine = slotEngine
            selectedVoskModel = availableVoskModels.length > 1 ? voskModel : ""
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
    <Tabs.Tab label={t('settings-general')} icon={Gear}>
        <Space h="sm" />
        <InputWrapper label="Нейросеть">
            <Text size="sm" color="gray">
                Для разговора и просьб, которых нет среди команд. Основная модель — Claude Haiku 5.5,
                резервные — Gemini и DeepSeek. Подключение через Polza AI или Kilo.
            </Text>
            <Space h="xs" />
            <NativeSelect
                data={[
                    { label: "Polza AI — без VPN, оплата рублями", value: "polza" },
                    { label: "Kilo — из России только через VPN, есть бесплатные модели", value: "kilo" }
                ]}
                variant="filled"
                bind:value={gateway}
            />
            <Space h="xs" />
            <Text size="sm" color="gray">
                {#if gateway === "polza"}
                    Ключ «sk-polza-…»: <a href="https://polza.ai" target="_blank">polza.ai</a> → личный кабинет,
                    пополнение от 100 ₽ по СБП или картой. Просьба стоит несколько копеек.
                {:else}
                    Без ключа — бесплатные модели, до 200 запросов в час. С ключом первыми отвечают платные:
                    быстрее и надёжнее. Ключ: <a href="https://app.kilo.ai" target="_blank">app.kilo.ai</a>
                    → Your Profile → внизу страницы (один на аккаунт).
                {/if}
                Кончатся деньги — Джарвис сам перейдёт на другой шлюз и бесплатные модели.
            </Text>
            <Space h="xs" />
            <Input
                type="password"
                autocomplete="off"
                placeholder={gateway === "polza" ? "Ключ Polza AI" : "Ключ Kilo (необязательно)"}
                variant="filled"
                value={gatewayKey}
                on:input={onKeyInput}
            />
            <Space h="sm" />
            <Switch label={freeOnly ? "Только бесплатные модели" : "Платные модели, если есть ключ"} bind:checked={freeOnly} />
        </InputWrapper>


        <Space h="xl" />
        <InputWrapper
            label="Зрение: ключ Google AI Studio"
            description="По просьбе «Джарвис, что у меня на экране?» снимок экрана уходит бесплатной модели Google — только тогда. Ключ: aistudio.google.com → Get API key. Из России Google работает только через VPN. Без ключа зрение выключено."
        >
            <Space h="xs" />
            <Input type="password" variant="filled" autocomplete="off" placeholder="AIza…" bind:value={visionKey} />
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
        <InputWrapper
            label="Что Джарвис помнит о вас"
            description="Нейросеть запоминает то, что вы рассказываете о себе, и учитывает в ответах. Голосом: «Джарвис, запомни, что…», «Джарвис, забудь…»."
        >
            <Space h="xs" />
            {#if memoryFacts.length === 0}
                <Text size="sm" color="dimmed">Пока ничего. Расскажите о себе — например, как вас зовут или какие игры любите.</Text>
            {:else}
                {#each memoryFacts as fact, index}
                    <div class="memory-fact">
                        <Text size="sm">{fact.text}</Text>
                        <Button size="xs" variant="subtle" color="gray" on:click={() => forgetFact(index)}>Забыть</Button>
                    </div>
                {/each}
            {/if}
        </InputWrapper>

        <Space h="xl" />
        <Text size="sm">Распознавание команд — GigaAM</Text>
        <Text size="xs" color="dimmed">
            {voiceServer.installed
                ? (voiceServer.running
                    ? (voiceServer.ready
                        ? (voiceServer.stt_available
                            ? "Голосовой сервер работает. Видеокарта: " + (voiceServer.gpu || "не найдена") + ". Распознавание: " + voiceServer.stt_engine + "."
                            : "Распознавание на сервере недоступно. Команды распознаёт встроенный резерв; перезапустите Джарвиса или переустановите голосовой сервер.")
                        : "Голосовой сервер запускается и загружает модели. Если ожидание затянулось, перезапустите Джарвиса.")
                    : "Голосовой сервер установлен, но сейчас не запущен. Запустите или перезапустите Джарвиса.")
                : "Установите голосовой сервер галочкой в установщике. Пока он недоступен, команды распознаёт встроенный резерв."}
        </Text>

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: "Jarvis New — ответы голосом", value: "http" },
                { label: "Не озвучивать, только уведомление", value: "none" }
            ]}
            label="Голос ответов нейросети"
            description="Ответы звучат голосом Jarvis New. Для синтеза нужна NVIDIA с CUDA или поддерживаемая AMD с ROCm."
            variant="filled"
            bind:value={ttsBackend}
        />

        {#if ttsBackend === "http" && voiceServer.tts_device}
            <Space h="sm" />
            <Text size="sm" color="dimmed">Голос: {voiceServer.tts_device}.</Text>
        {/if}

        {#if ttsBackend === "http" && voiceServer.running && voiceServer.tts_error}
            <Space h="sm" />
            <Alert title="Синтез голоса недоступен" color="yellow" variant="outline">{voiceServer.tts_error}</Alert>
        {/if}

        {#if assistantError}
            <Space h="sm" />
            <Alert title="Ошибка настроек" color="red" variant="outline">{assistantError}</Alert>
        {/if}

        <Space h="xl" />
        <div class="tools-row">
            <Button color="gray" radius="md" size="xs" uppercase on:click={openConfigFile}>Файл настроек</Button>
            <Button color="gray" radius="md" size="xs" uppercase on:click={collectLogs}>Собрать логи для отправки</Button>
            {#if updating}
                <Button color="green" radius="md" size="xs" uppercase disabled>Обновляю…</Button>
            {:else if update?.available}
                <Button color="green" radius="md" size="xs" uppercase disabled={updateBusy} on:click={installUpdate}>
                    Обновить до {update.latest}
                </Button>
            {:else}
                <Button color="gray" radius="md" size="xs" uppercase disabled={updateBusy} on:click={checkUpdate}>Проверить обновления</Button>
            {/if}
        </div>
        <UpdateProgress />
        {#if actionMessage}
            <Space h="sm" />
            <Text size="sm" color="gray">{actionMessage}</Text>
        {/if}
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
        {#if availableVoskModels.length > 1}
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
        {/if}

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
.memory-fact {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}
.tools-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

</style>
