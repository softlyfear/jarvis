---
name: add-voice-command
description: Добавить или изменить голосовую команду Джарвиса — пак в resources/commands (action, lua, voice), нативное действие в actions/ и инструмент для LLM. Использовать, когда просят «научи Джарвиса …», «добавь команду …», «сделай, чтобы по фразе … происходило …», когда Джарвис не понимает фразу или выполняет не ту команду, при правке command.toml, Action, from_voice_command или llm/tools.rs — даже если слово «команда» не прозвучало.
---

Read `.codex/skills/add-voice-command/SKILL.md` and follow its current procedure for this request.
Resolve its relative references from that canonical skill directory.
Use the Grok tools available in this session for equivalent operations; keep the documented
verification and authorization boundaries. Shared memory, tests and state
stay at their canonical paths. Apply `.grok/rules/git-workflow.md`
to commits. Do not apply `.claude/rules/git-workflow.md` in a Grok session.
