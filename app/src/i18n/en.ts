// English dictionary.
//
// Typed as `Record<Key, string>` where `Key` is derived from `zh.ts` — miss one
// entry and `tsc --noEmit` goes red. That is the whole point: a runtime lookup
// with a fallback would render a blank label, and a blank label on screen looks
// exactly like "there was never anything here".

import type { Key } from "./zh";

export const en: Record<Key, string> = {
  "nav.workbench": "Bench",
  "nav.presets": "Presets",
  "nav.devices": "Devices",
  "nav.history": "History",
  "nav.settings": "Settings",
  "app.loadingConfig": "Loading settings…",
  "app.gotIt": "Got it",
  "app.cancel": "Cancel",
  "app.save": "Save",
  "app.close": "Close",
  "app.refresh": "Refresh",
  "app.delete": "Delete",
  "app.edit": "Edit",
  "app.later": "Later",
  "app.retry": "Retry this panel",
  "app.copyDiagnostics": "Copy diagnostics",
  "app.blockFailed": "The \"{where}\" panel hit an error. Other pages are unaffected.",
  "app.blockFailedHint":
    "This is most likely our bug, not something you did wrong. Send us the diagnostics below.",

  "danger.stripTitle": "⚠ A danger-zone switch is on",
  "danger.stripSkip": " · inserting a card no longer asks",
  "danger.stripFormat": " · formats the card after copying",
  "danger.stripGo": "(click to review)",
  "danger.zoneTitle": "⚠ Danger zone",
  "danger.zoneHint": "These switches cause irreversible results. All off by default.",

  "workbench.currentProject": "Project",
  "workbench.noProject": "No project yet",
  "workbench.enabledPresets": "{n} preset(s) enabled",
  "workbench.running": "Copying",
  "workbench.idle": "Idle",
  "workbench.needProject":
    "No project yet. Create one under Settings → Projects so a card knows where to land.",
  "workbench.needPreset":
    "No preset enabled yet. Set one up under Presets, or just use \"Copy just once\".",
  "workbench.devices": "Devices",
  "workbench.devicesUsable": "{n} usable",
  "workbench.reading": "Reading local volumes…",
  "workbench.waitingCard": "Waiting for a card",
  "workbench.ignoredHint":
    "　·　{n} device(s) are on your ignore list and will stay silent when plugged in",
  "workbench.howToStart":
    "Inserting a card pops a confirmation automatically. If the card is already in, or you want another run, just hit \"Back up this card\" — both paths run the exact same orchestration.",
  "workbench.backupThis": "Back up this card",
  "workbench.copyOnce": "Copy just once…",
  "workbench.eject": "Eject safely",
  "workbench.format": "Format…",
  "workbench.canBeSource": "Usable as source",
  "workbench.inProgress": "In progress",
  "workbench.pause": "Pause",
  "workbench.resume": "Resume",
  "workbench.paused": "Paused",
  "workbench.speedUnknown": "Speed: —",
  "workbench.etaUnknown": "Remaining: —",
  "workbench.etaAbout": "About {d} left",
  "workbench.ejected": "{name} ejected safely — you can pull it out",

  "arrival.newDevice": "New device",
  "arrival.classifyHint":
    "Nothing is written before you identify it. This step cannot be skipped — acting automatically on an unknown device is not an acceptable risk.",
  "arrival.detected": "Card detected",
  "arrival.viaPreset": "Preset \"{name}\"",
  "arrival.toCopy": "{n} file(s) to copy · {size}",
  "arrival.skipped": ", {n} skipped",
  "arrival.willCopyTo": "Will copy to:",
  "arrival.start": "Start copying",
  "arrival.editThenCopy": "Adjust, then copy",

  "result.cancelled":
    "Task cancelled. Files already copied and verified will not be copied again.",
  "result.partial": "Partly failed: {ok} succeeded, {bad} failed",
  "result.ok": "Done: {n} file(s) · {size} · all verified",
  "result.okSkipped": " ({n} skipped — already copied and verified)",
  "result.failedFiles": "Failed files",
  "result.reason": "Reason",
  "result.viewReport": "View report",

  "adhoc.title": "Copy just once",
  "adhoc.preparing": "Preparing…",
  "adhoc.intro":
    "No preset is written this time. When it finishes you can remember this in one click — or leave nothing behind.",
  "adhoc.destLabel": "Copy to (required, up to 4)",
  "adhoc.destEmpty": "Nothing picked yet. This is the one thing nobody can decide for you.",
  "adhoc.addDest": "Add destination…",
  "adhoc.remove": "Remove",
  "adhoc.project": "Project",
  "adhoc.newProject": "Create a new one…",
  "adhoc.willCreate":
    "This project will be created for you. For destinations and path templates, see Settings → Projects.",
  "adhoc.verify": "Read-back verification",
  "adhoc.noVerifyWarn":
    "With verification off you will not know whether what landed is intact. One-off copies are no exception.",
  "adhoc.next": "Next",

  "sink.remember": "Remember this",
  "sink.no": "No thanks",
  "sink.scope": "Scope",
  "sink.scopeKind": "Devices of the same kind",
  "sink.scopeAny": "Any identified source device",
  "sink.alsoAs": "also record as",
  "sink.done":
    "Remembered. Next time this card is plugged in you'll get the confirmation straight away — edit it under Presets.",
  "sink.askNew": "From now on, when \"{device}\" is plugged in, copy it into \"{project}\"?",
  "sink.askDiverged":
    "This run differed from preset \"{preset}\" ({changed}). Make that the new default?",

  "settings.title": "Settings",
  "settings.language": "Language",
  "settings.languageAuto": "Follow system",
  "settings.copy": "Copying",
  "settings.verifyDefault": "Read-back verification",
  "settings.verifyHint":
    "Re-reads from the destination with caching bypassed and compares hashes. Turn it off and media write errors go undetected.",
  "settings.verifyOffWarn":
    "Verification is off — you will not know whether what landed is intact.",
  "settings.about": "About",
  "settings.guide": "Getting started",
  "settings.openGuide": "Open the guide",
  "settings.unsigned":
    "This build is not code-signed, so Windows will warn about an unknown publisher on first run. That is expected. Check the published SHA-256 checksum to confirm where the file came from.",
  "settings.neverDisable":
    "Never turn off your security software in order to run this program.",
  "settings.offline":
    "steadcopy does not go online: no account, no telemetry, no auto-update, and no background update check. Check the project page for new versions.",
};
