use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use jarvis_core::{audio, audio_buffer::AudioRingBuffer, audio_processing, commands, config, listener, recorder, stt, COMMANDS_LIST, intent, voices, ipc::{self, IpcEvent}, i18n, slots, actions, agent, tts, visual};
use rand::seq::SliceRandom;

use crate::should_stop;

// VAD state machine
#[derive(Debug, Clone, Copy, PartialEq)]
enum VadState {
    WaitingForVoice,
    VoiceActive,
}

pub fn start(text_cmd_rx: Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    main_loop(text_cmd_rx, rt)
}

fn main_loop(text_cmd_rx: Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    let frame_length: usize = 512;
    let sample_rate: usize = 16000;
    let mut frame_buffer: Vec<i16> = vec![0; frame_length];
    
    // ring buffer: keeps last 5 seconds of audio (pre-roll)
    let mut audio_buffer = AudioRingBuffer::new(5.0, frame_length, sample_rate);

    // VAD state
    let mut vad_state = VadState::WaitingForVoice;
    let mut silence_frames: u32 = 0;
    let mut was_speaking = false;
    
    // how many frames of silence before we consider speech ended
    // 1.5 seconds = 1.5 * (16000 / 512) ≈ 47 frames
    let silence_threshold: u32 = ((1.5 * sample_rate as f32) / frame_length as f32) as u32;
    
    voices::play_greet();

    match recorder::start_recording() {
        Ok(_) => info!("Recording started. Microphone: {}", 
            recorder::get_audio_device_name(recorder::get_selected_microphone_index())),
        Err(_) => {
            error!("Cannot start recording.");
            return Err(());
        }
    }

    ipc::send(IpcEvent::Idle);

    // ### WAKE WORD DETECTION LOOP
    'wake_word: loop {
        if should_stop() {
            info!("Stop signal received, shutting down...");
            voices::play_goodbye();
            ipc::send(IpcEvent::Stopping);
            break;
        }

        if let Some(out) = agent::bridge::process_next() {
            if let Some(question) = out.speech { speak(&question); }
            if out.chain {
                recognize_command(&mut frame_buffer, rt, frame_length, sample_rate, false);
            }
            audio_buffer.clear();
            stt::reset_wake_recognizer();
            continue 'wake_word;
        }

        if let Ok(text) = text_cmd_rx.try_recv() {
            process_text_command(&text, &rt);
            audio_buffer.clear();
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
            stt::reset_wake_recognizer();
            stt::reset_speech_recognizer();
            audio_processing::reset();
            continue 'wake_word;
        }

        recorder::read_microphone(&mut frame_buffer);

        // Jarvis is talking: his own voice must not wake him or become a command
        if audio::is_speaking() {
            was_speaking = true;
            continue 'wake_word;
        }
        if was_speaking {
            was_speaking = false;
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
            audio_buffer.clear();
            stt::reset_wake_recognizer();
            stt::reset_speech_recognizer();
            audio_processing::reset();
        }

        let processed = audio_processing::process(&frame_buffer);
        send_audio_level(&frame_buffer);
        
        match vad_state {
            VadState::WaitingForVoice => {
                // always buffer audio
                audio_buffer.push(&frame_buffer);
                
                if processed.is_voice {
                    // voice started! flush buffer to Vosk
                    info!("VAD: Voice started, flushing {} buffered frames", audio_buffer.len());
                    
                    for buffered_frame in audio_buffer.drain_all() {
                        stt::feed(&buffered_frame);
                        listener::data_callback(&buffered_frame);
                    }
                    
                    vad_state = VadState::VoiceActive;
                    silence_frames = 0;
                }
            }
            
            VadState::VoiceActive => {
                // dual-feed: speech recognizer gets frames in parallel with wake word detector
                stt::feed(&frame_buffer);

                // feed to wake word detector
                if let Some(_keyword_index) = listener::data_callback(&frame_buffer) {
                    // WAKE WORD DETECTED!
                    info!("Wake word activated!");
                    ipc::send(IpcEvent::WakeWordDetected);
                    
                    stt::reset_wake_recognizer();
                    audio_processing::reset();

                    // brief sniff to keep feeding STT while transitioning
                    let sniff_frames = ((0.3 * sample_rate as f32) / frame_length as f32) as u32;
                    for _ in 0..sniff_frames {
                        recorder::read_microphone(&mut frame_buffer);
                        audio_processing::process(&frame_buffer);
                        stt::feed(&frame_buffer);
                    }

                    ipc::send(IpcEvent::Listening);
                    recognize_command(&mut frame_buffer, &rt, frame_length, sample_rate, true);

                    // reset state after command
                    vad_state = VadState::WaitingForVoice;
                    silence_frames = 0;
                    audio_buffer.clear();
                    stt::reset_wake_recognizer();
                    stt::reset_speech_recognizer(); // NOW reset, after command is done
                    audio_processing::reset();
                    ipc::send(IpcEvent::Idle);
                    
                    continue 'wake_word;
                }
                
                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    
                    if silence_frames > silence_threshold {
                        debug!("VAD: Silence timeout, returning to wait state");
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        stt::reset_wake_recognizer();
                        stt::reset_speech_recognizer(); // reset since we were dual-feeding
                    }
                }
            }
        }
    }

    recorder::stop_recording().ok();
    ipc::send(IpcEvent::Stopping);

    Ok(())
}


// Voice recognition for command after wake word
fn recognize_command(
    frame_buffer: &mut [i16],
    rt: &tokio::runtime::Runtime,
    frame_length: usize,
    sample_rate: usize,
    prefed_audio: bool
) {
    let mut audio_buffer = AudioRingBuffer::new(2.0, frame_length, sample_rate);
    let mut vad_state = if prefed_audio {
        VadState::VoiceActive
    } else {
        VadState::WaitingForVoice
    };
    let mut silence_frames: u32 = 0;
    let mut start = SystemTime::now();
    let mut first_recognition = prefed_audio;
    let mut was_speaking = false;
    
    // longer silence threshold for commands (user might pause to think)
    // 5 seconds
    let silence_threshold: u32 = ((5.0 * sample_rate as f32) / frame_length as f32) as u32;
    
    loop {
        if crate::should_stop() {
            return;
        }
        
        recorder::read_microphone(frame_buffer);

        // skip the assistant's own reply sounds and speech
        if audio::is_speaking() {
            was_speaking = true;
            continue;
        }
        if was_speaking {
            was_speaking = false;
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
            audio_buffer.clear();
            stt::reset_speech_recognizer();
            audio_processing::reset();
        }

        let processed = audio_processing::process(frame_buffer);
        send_audio_level(frame_buffer);
        
        match vad_state {
            VadState::WaitingForVoice => {
                audio_buffer.push(frame_buffer);
                
                if processed.is_voice {
                    // flush buffer to STT
                    for buffered_frame in audio_buffer.drain_all() {
                        stt::feed(&buffered_frame);
                    }
                    vad_state = VadState::VoiceActive;
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    
                    if silence_frames > silence_threshold {
                        info!("Long silence detected, returning to wake word mode.");
                        return;
                    }
                }
            }
            
            VadState::VoiceActive => {
                // feed to STT
                if let Some(mut recognized_voice) = stt::recognize(frame_buffer, false) {
                    info!("Recognized voice: {}", recognized_voice);
                    
                    ipc::send(IpcEvent::SpeechRecognized {
                        text: recognized_voice.clone(),
                    });
                    
                    recognized_voice = recognized_voice.to_lowercase();

                    // the phrase that woke him up: "ничего не произошло, Джарвис, закрой телеграм"
                    // is a command after the name; "молодец, Джарвис" stays whole
                    if first_recognition {
                        if let Some(rest) = actions::text::after_address(&recognized_voice).filter(|r| !r.trim().is_empty()) {
                            recognized_voice = rest;
                        }
                    }

                    // check if wake word repeated (reactivate)
                    let stripped = actions::text::strip_address(&recognized_voice);
                    let contains_wake = stripped != recognized_voice;

                    if contains_wake {
                        // strip the wake word
                        let remaining = stripped.trim();

                        if remaining.is_empty() {
                            if first_recognition {
                                // leftover wake word from dual-feed, just discard it
                                info!("Discarding initial wake word from prefed audio");
                                first_recognition = false;
                                stt::reset_speech_recognizer();
                                voices::play_reply();
                                vad_state = VadState::WaitingForVoice;
                                silence_frames = 0;
                                start = SystemTime::now();
                                audio_buffer.clear();
                                continue;
                            }

                            // just wake word, no command - reactivate
                            info!("Wake word repeated during chaining, reactivating...");
                            voices::play_reply();
                            stt::reset_speech_recognizer();
                            ipc::send(IpcEvent::Listening);
                            
                            vad_state = VadState::WaitingForVoice;
                            silence_frames = 0;
                            start = SystemTime::now();
                            audio_buffer.clear();
                            continue;
                        } else {
                            // wake word + command in one phrase - execute the command part
                            info!("Wake word + command during chaining: '{}'", remaining);
                            recognized_voice = remaining.to_string();
                            // fall through to command execution below
                        }
                    }

                    first_recognition = false;
                    
                    // filter activation phrases
                    // for tbr in config::ASSISTANT_PHRASES_TBR {
                    //     recognized_voice = recognized_voice.replace(tbr, "");
                    // }
                    // Preserve command verbs and addresses inside names or dictated text.
                    recognized_voice = actions::text::strip_address(&recognized_voice);

                    recognized_voice = recognized_voice.trim().to_string();
                    
                    // short answers ("да") are valid while a confirmation is pending
                    if recognized_voice.chars().count() < 3 && !actions::confirm::has_pending() && !actions::dialog::has_pending() {
                        debug!("Ignoring too short recognition: '{}'", recognized_voice);
                        continue;
                    }

                    if recognized_voice.is_empty() {
                        continue;
                    }
                    
                    // execute command and check if we should chain
                    audio::take_interrupted(); // a stale cut-off from before must not silence this answer
                    let should_chain = execute_command(&recognized_voice, rt);
                    // cut off with "Джарвис": the next command follows right away
                    let should_chain = audio::take_interrupted() || should_chain;
                    
                    if should_chain {
                        // chain: reset and continue listening
                        info!("Chaining enabled, continuing to listen...");
                        stt::reset_speech_recognizer();
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        start = SystemTime::now();
                        audio_buffer.clear();
                        ipc::send(IpcEvent::Listening);
                        continue;
                    } else {
                        // no chain: return to wake word
                        info!("No chain, returning to wake word mode.");
                        return;
                    }
                }
                
                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    
                    if silence_frames > silence_threshold {
                        info!("Long silence detected, returning to wake word mode.");
                        return;
                    }
                }
            }
        }
        
        // timeout
        if let Ok(elapsed) = start.elapsed() {
            if elapsed > config::CMS_WAIT_DELAY {
                info!("Command timeout, returning to wake word mode.");
                return;
            }
        }
    }
}


fn process_text_command(text: &str, rt: &tokio::runtime::Runtime) {
    info!("Processing text command: {}", text);
    
    ipc::send(IpcEvent::SpeechRecognized { text: text.to_string() });
    
    let filtered = actions::text::strip_address(text);
    let filtered = filtered.trim();
    
    if filtered.is_empty() {
        ipc::send(IpcEvent::Idle);
        return;
    }
    
    // text commands never chain; a reply cut off with "Джарвис" must not silence the next question
    audio::take_interrupted();
    execute_command(filtered, rt);
    audio::take_interrupted();
}


// Execute command, returns true if chaining should continue
fn execute_command(text: &str, rt: &tokio::runtime::Runtime) -> bool {
    recorder::discard_pending_audio();
    agent::bridge::expire();
    agent::has_pending(); // drops an expired or cancelled OpenClaw confirmation
    if let Some(result) = agent::with_action_lock(|| actions::dialog::answer(text)) {
        match result {
            Ok(out) => {
                agent::remember_command(text, &out.report);
                if !out.chain && (agent::has_pending() || agent::bridge::has_pending()) {
                    return finish_agent_action(&out.report, true, rt);
                }
                if let Some(speech) = out.speech { speak(&speech); }
                ipc::send(IpcEvent::CommandExecuted { id: "dialog".into(), success: true });
                return out.chain;
            }
            Err(e) => {
                if agent::has_pending() || agent::bridge::has_pending() { return finish_agent_action(&format!("ошибка: {}", e), false, rt); }
                speak(&format!("Не получилось: {}", e));
                ipc::send(IpcEvent::Error { message: e.to_string() });
                return false;
            }
        }
    }
    let (answer, confirmed_result) = agent::with_action_lock(|| {
        let answer = actions::confirm::answer(text);
        let result = match &answer {
            actions::confirm::Answer::Confirmed(action) => Some(action.execute()),
            _ => None,
        };
        (answer, result)
    });
    match answer {
        actions::confirm::Answer::Confirmed(_) => {
            info!("Confirmed action");
            match confirmed_result.expect("confirmed action result") {
                Ok(out) => {
                    if agent::has_pending() || agent::bridge::has_pending() { return finish_agent_action(&out.report, true, rt); }
                    agent::remember_command("Подтверждённое действие", &out.report);
                    voices::play_ok();
                    ipc::send(IpcEvent::CommandExecuted { id: "confirmed_action".into(), success: true });
                }
                Err(e) => {
                    if agent::has_pending() || agent::bridge::has_pending() { return finish_agent_action(&format!("ошибка: {}", e), false, rt); }
                    speak(&format!("Не получилось: {}", e));
                    ipc::send(IpcEvent::Error { message: e.to_string() });
                }
            }
            ipc::send(IpcEvent::Idle);
            return false;
        }
        actions::confirm::Answer::Cancelled => {
            if agent::has_pending() || agent::bridge::has_pending() { return finish_agent_action("не выполнено: пользователь отменил действие", false, rt); }
            speak("Отменено.");
            ipc::send(IpcEvent::Idle);
            return false;
        }
        actions::confirm::Answer::Unrelated | actions::confirm::Answer::NoPending => {
            if agent::has_pending() && !actions::dialog::has_pending() { agent::abandon_pending(); }
            if agent::bridge::has_pending() && !actions::dialog::has_pending() { agent::bridge::finish("не выполнено: подтверждение отменено или истекло", false); }
        }
    }

    let commands_list = match COMMANDS_LIST.get() {
        Some(c) => c,
        None => {
            ipc::send(IpcEvent::Error { message: "Commands not loaded".to_string() });
            ipc::send(IpcEvent::Idle);
            return false;
        }
    };
    
    let cmd_result = if let Some((intent_id, confidence)) = 
        rt.block_on(intent::classify(text)) 
    {
        info!("Intent recognized: {} (confidence: {:.2})", intent_id, confidence);
        intent::get_command_by_intent(commands_list, &intent_id)
            .filter(|(_, cmd)| commands::supports_direct_command(text, cmd))
            .or_else(|| commands::fetch_command(text, commands_list))
    } else {
        info!("Intent not recognized, trying levenshtein fallback...");
        commands::fetch_command(text, commands_list)
    };
    
    if let Some((cmd_path, cmd_config)) = cmd_result {
        info!("Command found: {:?}", cmd_path);

        // native actions: speak their questions/errors, hand unknown names to the LLM
        if cmd_config.cmd_type == "action" {
            let templates = cmd_config.get_phrases(&i18n::get_language());
            let result = actions::from_voice_command(&cmd_config.action, text, &templates, &cmd_config.args)
                .and_then(|a| agent::execute_action(a, &agent::RequestControl::default()));

            match result {
                Ok(outcome) => {
                    info!("Action {} done: {}", cmd_config.action, outcome.report);
                    agent::remember_command(text, &outcome.report);
                    match &outcome.speech {
                        Some(speech) => speak(speech),
                        None => voices::play_random_from(cmd_config.get_sounds(&i18n::get_language()).as_slice()),
                    }
                    ipc::send(IpcEvent::CommandExecuted { id: cmd_config.id.clone(), success: true });
                    ipc::send(IpcEvent::Idle);
                    return outcome.chain;
                }
                Err(actions::ActionError::NotFound(reason)) => {
                    info!("Action {} found nothing ({}), asking LLM", cmd_config.action, reason);
                    return ask_llm(text, Some(&reason), rt);
                }
                Err(e) => {
                    error!("Action {} failed: {}", cmd_config.action, e);
                    voices::play_error();
                    speak(&e.to_string());
                    ipc::send(IpcEvent::CommandExecuted { id: cmd_config.id.clone(), success: false });
                    ipc::send(IpcEvent::Error { message: e.to_string() });
                    ipc::send(IpcEvent::Idle);
                    return false;
                }
            }
        }
        
        // extract slots if needed
        let extracted_slots = if !cmd_config.slots.is_empty() {
            let s = slots::extract(text, &cmd_config.slots);
            if !s.is_empty() {
                info!("Extracted slots: {:?}", s);
            }
            Some(s)
        } else {
            None
        };

        match commands::execute_command(&cmd_path, &cmd_config, Some(&text), extracted_slots.as_ref()) {
            Ok(chain) => {
                info!("Command executed successfully");
                agent::remember_command(text, &cmd_config.id);
                // voices::play_ok();
                voices::play_random_from(cmd_config.get_sounds(&i18n::get_language()).as_slice());
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: true,
                });
                ipc::send(IpcEvent::Idle);
                return chain; // return chain status from command
            }
            Err(msg) => {
                error!("Error executing command: {}", msg);
                voices::play_error();
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: false,
                });
                ipc::send(IpcEvent::Error { message: msg.to_string() });
            }
        }
    } else {
        info!("No command found for: {}", text);
        return ask_llm(text, None, rt);
    }
    
    ipc::send(IpcEvent::Idle);
    false // no chain on error or not found
}

// hybrid fallback: anything the built-in commands could not handle goes to the LLM
fn ask_llm(text: &str, hint: Option<&str>, rt: &tokio::runtime::Runtime) -> bool {
    if !agent::is_configured() {
        info!("LLM is not configured, command not found");
        voices::play_not_found();
        if let Some(h) = hint {
            speak(h);
        }
        ipc::send(IpcEvent::Error { message: format!("Command not found: {}", text) });
        ipc::send(IpcEvent::Idle);
        return false;
    }

    run_agent(agent::AgentRequest::text(text), rt)
}

fn finish_agent_action(report: &str, success: bool, rt: &tokio::runtime::Runtime) -> bool {
    if agent::bridge::finish(report, success) {
        if success { voices::play_ok(); } else { speak(report); }
        return false;
    }
    run_agent(agent::AgentRequest::continuation(report), rt)
}

fn run_agent(request: agent::AgentRequest, rt: &tokio::runtime::Runtime) -> bool {
    match wait_for_agent(request, rt) {
        Ok((reply, mut preview)) => {
            info!("LLM reply: {}", reply.speech);
            actions::platform::notify("Джарвис", &reply.speech);
            if let Some((sentence, bytes)) = preview.take(&reply.speech) {
                ipc::send(IpcEvent::Speaking { active: true });
                let barge_in = jarvis_core::assistant_config::get().tts.barge_in && !sentence.to_lowercase().contains("джарвис");
                let played = if barge_in { tts::play_prepared(&bytes, &wait_for_wake_word) }
                    else { tts::play_prepared(&bytes, &|d| std::thread::sleep(d)) };
                ipc::send(IpcEvent::Speaking { active: false });
                if played.is_ok() { speak(reply.speech.strip_prefix(&sentence).unwrap_or("").trim()); }
                else { speak(&reply.speech); }
            } else { speak(&reply.speech); }
            ipc::send(IpcEvent::CommandExecuted { id: "llm".into(), success: reply.success });
            ipc::send(IpcEvent::Idle);
            reply.chain
        }
        Err(agent::AgentError::Cancelled) => { ipc::send(IpcEvent::WakeWordDetected); true }
        Err(e) => {
            error!("Agent failed: {}", e);
            voices::play_error();
            speak(&e.to_string());
            ipc::send(IpcEvent::Error { message: format!("LLM: {}", e) });
            ipc::send(IpcEvent::Idle);
            false
        }
    }
}


// The worker waits for HTTP while this thread keeps listening for cancellation.
fn cancel_agent_request(control: &agent::RequestControl) {
    control.cancel();
    actions::confirm::clear();
    actions::dialog::clear();
    agent::abandon_pending();
    agent::bridge::finish("не выполнено: запрос отменён", false);
}

fn wait_for_agent(request: agent::AgentRequest, rt: &tokio::runtime::Runtime) -> Result<(agent::AgentReply, agent::speech::SpeechPreview), agent::AgentError> {
    let control = agent::RequestControl::default();
    let worker_control = control.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = agent::handle(&request, &worker_control, &|event| { let _ = event_tx.send(event); });
        let _ = tx.send(result);
    });
    recorder::discard_pending_audio();
    stt::reset_wake_recognizer();
    let started = std::time::Instant::now();
    let mut announced = false;
    let mut preview = agent::speech::SpeechPreview::default();
    let mut frame = vec![0; 512];
    loop {
        if should_stop() { cancel_agent_request(&control); return Err(agent::AgentError::Cancelled); }
        if let Some(out) = agent::bridge::process_next() {
            if let Some(question) = out.speech { speak(&question); }
            if out.chain { recognize_command(&mut frame, rt, 512, 16000, false); }
            recorder::discard_pending_audio(); stt::reset_wake_recognizer();
        }
        match rx.try_recv() {
            Ok(result) => { stt::reset_wake_recognizer(); return result.map(|reply| (reply, preview)); }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err(agent::AgentError::ProviderError),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        for event in event_rx.try_iter() {
            match event {
                agent::AgentEvent::ToolCall { name } => { info!("Agent tool: {}", name); preview.reset(); }
                agent::AgentEvent::TextDelta(text) => preview.push(&text, &control),
                agent::AgentEvent::BackendChanged | agent::AgentEvent::Error(_) => preview.reset(),
                _ => {}
            }
        }
        if !announced && started.elapsed().as_secs() >= 4 {
            announced = true;
            speak("Выполняю.");
            if audio::take_interrupted() { cancel_agent_request(&control); return Err(agent::AgentError::Cancelled); }
            recorder::discard_pending_audio(); stt::reset_wake_recognizer();
        }
        recorder::read_microphone(&mut frame);
        send_audio_level(&frame);
        if !audio::is_speaking() && listener::data_callback(&frame).is_some() {
            cancel_agent_request(&control);
            recorder::discard_pending_audio(); stt::reset_wake_recognizer();
            return Err(agent::AgentError::Cancelled);
        }
    }
}

// synthesized speech, with GUI notifications so the orb can animate
fn speak(text: &str) {
    ipc::send(IpcEvent::Speaking { active: true });
    // a reply that names him would cut itself off through the speakers
    // cut off already: the rest of this answer is not wanted
    if audio::is_interrupted() {
        ipc::send(IpcEvent::Speaking { active: false });
        return;
    }
    let barge_in = jarvis_core::assistant_config::get().tts.barge_in && !text.to_lowercase().contains("джарвис");
    if barge_in {
        tts::speak_with(text, &wait_for_wake_word);
    } else {
        tts::speak(text);
    }
    ipc::send(IpcEvent::Speaking { active: false });
}

// while the reply plays, only the wake word detector listens: "Джарвис" stops the speech
fn wait_for_wake_word(d: std::time::Duration) {
    let end = std::time::Instant::now() + d;
    let mut frame: Vec<i16> = vec![0; 512];
    recorder::discard_pending_audio();
    stt::reset_wake_recognizer();
    while std::time::Instant::now() < end {
        recorder::read_microphone(&mut frame);
        if listener::data_callback(&frame).is_some() {
            info!("Wake word during speech, stopping the reply");
            audio::stop_speaking();
            stt::reset_wake_recognizer();
            ipc::send(IpcEvent::WakeWordDetected);
            return;
        }
    }
    stt::reset_wake_recognizer();
}

fn send_audio_level(frame: &[i16]) {
    let f = visual::analyze(frame);
    ipc::send(IpcEvent::AudioLevel { level: f.level, bands: f.bands });
}


// log the reason, show it to the user, exit
pub fn fatal(message: &str) -> ! {
    error!("FATAL: {}", message);
    show_error(message);
    ipc::send(IpcEvent::Stopping);
    std::process::exit(1);
}

pub fn show_error(message: &str) {
    let log_hint = jarvis_core::APP_LOG_DIR
        .get()
        .map(|d| format!("\n\nПодробности: {}", d.join(config::LOG_FILE_NAME).display()))
        .unwrap_or_default();
    let text = format!("{}{}", message, log_hint);

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() };
        let body = wide(&text);
        let title = wide("Джарвис");
        unsafe {
            winapi::um::winuser::MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                winapi::um::winuser::MB_OK | winapi::um::winuser::MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("{}", text);
}


pub fn close(code: i32) {
    info!("Closing application.");
    voices::play_goodbye();
    ipc::send(IpcEvent::Stopping);
    std::process::exit(code);
}
