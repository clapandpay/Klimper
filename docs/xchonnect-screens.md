# Xchonnect screens

The design pass for the Xchonnect screens (clapandpay **TASK-069**), and the contract the
screens that implement it keep (TASK-072, TASK-080, TASK-083, TASK-084, TASK-085).

Pairing is where phishing happens and the approval screen is where money leaves. These
screens are the security surface of the feature, so this document is prescriptive about
what is shown, in what words, and what the user is **not** offered.

Normative behaviour is xchonnect's `docs/spec/xchonnect-spec.md`; what a wallet owes the
user is its `docs/wallet-integration.md` (§1, §5, §6 and the MUST checklist). The typed
command surface is `src/klimper/xchonnect.ts`.

## The four rules nothing may drop

1. **Both domain forms when they differ, and every warning.** `domainUnicode` is what the
   user reads; `domainAscii` is what the computer resolved. When `domainDiffers`, both are
   on screen, and every entry of `warnings` has its own line with its own plain-language
   sentence.
2. **No continue-anyway on a failure.** A failed origin verification, a used pairing code
   or an expired one ends the flow. The only controls are "Start over" and "Cancel". There
   is no control that proceeds, not behind a confirmation and not behind a details
   disclosure.
3. **Only the wallet's own simulation is rendered.** `prompt.summary` and `prompt.plan`
   come from local CLVM execution. A dApp's claims about amounts, recipients or purpose
   are not evidence and are not rendered — the one exception is the message of a
   `signMessage` request, which _is_ the object being signed and is shown inside a labelled
   quoted block, never as an amount or a recipient.
4. **One confirmation per signature.** A `kind: 'signature'` approval is answered on its
   own. They are never batched, never answered by one button, and never pre-answered by
   approving the request.

## Design language

Klimper is deliberately cleaner than Sage: Sage's `AdvancedTransactionSummary` answers
"what does this transaction contain", and this screen answers "what happens to my money".
The second question has a short answer, and the long one goes behind a disclosure.

| Role                 | Token                                  | Used for                                                               |
| -------------------- | -------------------------------------- | ---------------------------------------------------------------------- |
| Verdict line         | `text-2xl font-semibold`               | the one number the decision turns on: the net effect per asset         |
| Loss                 | `text-destructive`                     | a negative net, a fee                                                  |
| Gain                 | `text-foreground`                      | a positive net — never `text-green`, so "safe" is never a colour claim |
| Soft                 | `text-muted-foreground`                | anything conditional, informational or claimed by the dApp             |
| Danger panel         | `border-destructive` + `AlertTriangle` | `AGG_SIG_UNSAFE`, an unknown contract, a homograph warning             |
| Verified-domain slab | `text-xl font-semibold break-all`      | the domain, always the largest text on a pairing screen                |
| SAS                  | `text-5xl font-mono tracking-[0.3em]`  | the six digits                                                         |
| Claimed text         | `text-muted-foreground` + "it claims"  | `dappName`, the `signMessage` body                                     |

Both themes come from the existing theme tokens rather than literal colours, so light and
dark follow the app (`theme-o-rama`); `text-destructive` and `border-destructive` are the
only semantic colour used, and every danger panel also carries an icon and a word, so the
warning survives a colour-blind reader and a monochrome screenshot.

TASK-004's tokens do not exist yet (that task is open), and TASK-062's Klimper theme is in
progress. These screens therefore use Sage's existing token names, which both of those
tasks redefine rather than rename; a theme change moves these screens with it. No literal
hex value appears in any Xchonnect screen.

## Screens and states

### 1. Connected dApps — `/xchonnect` (TASK-084)

| State                 | What is shown                                                                                                                                         |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| empty                 | "No dApps are connected." plus what a connection is and the one button that starts one                                                                |
| a session             | verified domain (both forms when they differ), claimed name as claimed, when it was connected, key epoch, listening or not, methods, limits, switches |
| `needsRepair`         | the card is marked unusable, with "Connect again" — no action that would use it                                                                       |
| `warnings`            | a danger line per warning, on the card                                                                                                                |
| dangerous switches on | a warning line naming which one                                                                                                                       |
| no wallet open        | the list is empty and says the wallet has to be unlocked first                                                                                        |

Actions per session: **Permissions** (screen 5) and **End connection**. Ending is
immediate and needs no confirmation dialog — it is the safe direction.

### 2. Connect a dApp — `/xchonnect/connect` (TASK-072, TASK-085)

| Stage        | What is shown                                                                                                                                              |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| entry        | scan or paste the code; on desktop: paste from the clipboard or choose a QR image; on mobile: the camera. The field accepts `xchonnect:v1?…` and `/pair#…` |
| checking     | one line, no spinner theatre: "Checking the website…"                                                                                                      |
| verified     | the domain slab, warnings, what the dApp may ask for, the single address shared, the network, the relay, and "nothing is signed automatically"             |
| failed       | the reason, "there is no way to continue past this", and only Start over / Cancel                                                                          |
| SAS          | six digits, large; "the website shows the same six digits"; both answers only once the dApp has answered                                                   |
| waiting      | "Waiting for the website…" with the deadline                                                                                                               |
| used/expired | the same dead end as `failed`, with the copy below                                                                                                         |

A link that arrives from the OS is handed over in memory, never through the app URL: the
app uses a hash router, so a fragment in the URL would both break routing and write the
pairing secret into the history stack.

### 3. Approval — `/xchonnect/request` (TASK-080)

Opened by the `approval` event, by a `/req` link, and rebuilt from `pendingApprovals()`
after a reload. One approval is on screen at a time; the rest are counted ("2 more
waiting") and never rendered in a way that could be answered by accident.

```
┌──────────────────────────────────────────┐
│ pengui.xyz asks you to sign              │  verified domain, ASCII form too when it differs
│ signCoinSpends · expires in 4:31         │
├──────────────────────────────────────────┤
│ You pay          − 1.5 XCH               │  the verdict line, per asset
│ You receive        + 0 XCH               │
│ Only if the counterparty completes:      │  never added to a total
│                  + 250 CAT 1a2b3c4d…     │
│ Network fee        0.0000005 XCH         │
├──────────────────────────────────────────┤
│ ⚠ Unknown contract                       │  danger panel, puzzle hash in full
│ ⚠ Unsafe signature                       │  danger panel, red, the loudest thing present
├──────────────────────────────────────────┤
│ ▸ Everything Klimper computed            │  disclosure: the whole prompt, verbatim JSON
├──────────────────────────────────────────┤
│ [ Approve ]  [ Reject ]                  │
└──────────────────────────────────────────┘
```

- **Approve** on a prompt that carries an unknown puzzle opens the second confirmation,
  which shows the puzzle hash and has its own Approve.
- **Dismiss** (navigating away, closing) is a rejection, not a postponement: the plugin
  treats five minutes of silence as a rejection, and the screen says so.
- A `kind: 'signature'` approval is its own card: "Authorise signature 2 of 3", the public
  key, and one Approve / Reject. The count is informational — the next signature asks
  again.

### 4. Lock screen and notifications

No amount, no address, no asset, no domain in Unicode form. The whole copy is:

> **Klimper** — A connected website is asking you to sign something. Open Klimper to see
> what it is.

That is also what an encrypted notification preview may say (TASK-088); a preview that is
missing, malformed, unauthenticated or expired produces exactly this text, so the user
cannot tell the difference and no preview can ever drive a decision.

### 5. Permissions and limits — `/xchonnect/dapp/:sessionId` (TASK-083)

Allowed methods (as checkboxes over the methods the session was granted), the exposed key,
a per-request and a per-day limit per asset, and the two dangerous switches. Defaults are
spec 9.3: the CHIP-0002 required methods, **one fresh key**, no auto-approval, both
switches off, no limits.

Each dangerous switch opens its own confirmation dialog, which names the consequence in
one sentence and requires the explicit button; cancelling leaves the switch off. Turning
one **off** is immediate.

A request refused by a limit never reaches a prompt: wallet-kit refuses it and the dApp
gets the error. The screen says that in the limits section, so a user who wonders why
nothing appeared can find the reason.

## Rendering table

Every field the view may read, and nothing else. `src/klimper/xchonnectPrompt.ts` reads
exactly this list by explicit key and validated type; any other key in the prompt JSON —
including anything a dApp managed to influence — is dropped before the view sees it.

| Prompt field                                 | Shown as                                                                                 | Copy                                      |
| -------------------------------------------- | ---------------------------------------------------------------------------------------- | ----------------------------------------- |
| `summary.assets[].asset`                     | asset name: `XCH`, or `CAT <first 8 hex of asset_id>…`; never a dApp-supplied name       | —                                         |
| `summary.assets[].net` < 0                   | the verdict line, `text-destructive`, absolute value                                     | "You pay"                                 |
| `summary.assets[].net` > 0                   | the verdict line                                                                         | "You receive"                             |
| `summary.assets[].net` = 0                   | one line, soft                                                                           | "No change"                               |
| `summary.assets[].conditional_received`      | its own soft line under the asset, **never summed into `net`** and never called received | "Only if the counterparty completes"      |
| `summary.assets[].sent` / `received`         | inside the disclosure only — the gross figures mislead on their own                      | —                                         |
| `summary.implied_fee`                        | a line under the assets                                                                  | "Network fee"                             |
| `summary.reserve_fee`                        | a line under the assets, when non-zero                                                   | "Fee reserved by the spend"               |
| `summary.unknown_puzzles[]`                  | danger panel, every puzzle hash in full and selectable; gates a second confirmation      | "Unknown contract"                        |
| `summary.time_locks`                         | one line per lock, with the absolute time                                                | "Not spendable before" / "Expires"        |
| `plan.ours[].is_unsafe` or `plan.has_unsafe` | the loudest danger panel on the screen, red, icon, never collapsed                       | "Unsafe signature"                        |
| `plan.ours[].public_key`                     | in the per-signature approval, monospace, full                                           | "Signing key"                             |
| `plan.ours.length`                           | a soft line                                                                              | "Signatures you would produce"            |
| `plan.foreign`                               | count only, inside the disclosure                                                        | —                                         |
| `partial`                                    | a panel explaining that this is one side of a multi-party spend                          | "This is one half of a trade"             |
| `binding.all_bound` = true                   | inside that panel                                                                        | "Klimper checked the other side is bound" |
| `binding.all_bound` = false                  | danger panel — this should have been refused upstream                                    | "Not bound — do not sign"                 |
| `binding.bound_payments[]`                   | what the signature depends on receiving, per payment                                     | "Your signature only pays out if…"        |
| `message_text` (signMessage)                 | a labelled quoted block, monospace, control characters escaped, length shown             | "The website asks you to sign this text"  |
| `message_hex` (signMessage)                  | the same block when there is no text form                                                | "…this data"                              |
| `public_key` (signMessage)                   | monospace, full                                                                          | "With this key"                           |
| anything else                                | **dropped**                                                                              | —                                         |

The "Everything Klimper computed" disclosure prints the **parsed** view, not the prompt
JSON. Printing the JSON would put every key the prompt happens to carry on the screen,
which is the one thing this screen must not do — a field a dApp could influence would be
rendered after all, just inside a disclosure. (That is not hypothetical care: the test
that asserts no dApp string reaches the screen failed on exactly this.)

`conditional_received` is the field this table exists for. It is a number the user would
read as income; it is income only if somebody else acts. It is never added to a total, never
rendered next to `net` without its sentence, and never styled like a gain.

## Copy, English and German

The security-critical strings. German is the second launch language (`src/locales/de-DE`);
these are the translations to put in the catalogue, written for a reader who does not know
what a puzzle hash is.

| Key                    | English                                                                                                                  | German                                                                                                                                          |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| verified domain        | This domain proved it owns the key that signed the code. Only the domain is verified — the name below is what it claims. | Diese Domain hat bewiesen, dass ihr der Schlüssel gehört, der den Code signiert hat. Geprüft ist nur die Domain — der Name ist eine Behauptung. |
| non-ASCII warning      | This domain uses letters outside the English alphabet. Check it character by character.                                  | Diese Domain benutzt Buchstaben außerhalb des englischen Alphabets. Prüfen Sie sie Zeichen für Zeichen.                                         |
| mixed scripts          | This domain mixes alphabets, which is how look-alike domains are built. Check it carefully.                              | Diese Domain mischt Alphabete — so werden Domains gebaut, die echten zum Verwechseln ähnlich sehen. Prüfen Sie genau.                           |
| failure dead end       | There is no way to continue past this. Ask the website for a new code.                                                   | Hier geht es nicht weiter. Lassen Sie sich von der Website einen neuen Code geben.                                                              |
| code already used      | This code was already used, possibly on another device. Ask the website for a new one.                                   | Dieser Code wurde schon benutzt, möglicherweise auf einem anderen Gerät. Lassen Sie sich einen neuen geben.                                     |
| SAS                    | The website shows the same six digits. If they are different, somebody is in the middle.                                 | Die Website zeigt die gleichen sechs Ziffern. Wenn sie abweichen, sitzt jemand dazwischen.                                                      |
| simulated, not claimed | This is what Klimper worked out by running the spend itself, not what the website says it does.                          | Das hat Klimper selbst ausgerechnet, indem es die Ausgabe ausgeführt hat — nicht, was die Website behauptet.                                    |
| conditional            | Only if the counterparty completes                                                                                       | Nur wenn die Gegenseite ihren Teil erfüllt                                                                                                      |
| unsafe signature       | This signature would be valid in any spend, not just this one. Reject it unless you know exactly why it is needed.       | Diese Signatur wäre in jeder Ausgabe gültig, nicht nur in dieser. Lehnen Sie ab, außer Sie wissen genau, warum sie nötig ist.                   |
| unknown contract       | Klimper does not recognise this contract. Nobody can tell you what it does with your coins.                              | Klimper kennt diesen Vertrag nicht. Niemand kann Ihnen sagen, was er mit Ihren Coins macht.                                                     |
| unbound partial        | Not bound — do not sign. Your signature would not be tied to the payment you are expecting.                              | Nicht gebunden — nicht signieren. Ihre Signatur wäre nicht an die erwartete Zahlung gebunden.                                                   |
| per-signature          | Authorise signature {index} of {total}                                                                                   | Signatur {index} von {total} freigeben                                                                                                          |
| dismiss is reject      | Leaving this screen rejects the request.                                                                                 | Wenn Sie diesen Bildschirm verlassen, wird die Anfrage abgelehnt.                                                                               |
| notification           | A connected website is asking you to sign something. Open Klimper to see what it is.                                     | Eine verbundene Website möchte, dass Sie etwas signieren. Öffnen Sie Klimper, um zu sehen, was.                                                 |
| limit refused          | A request over a limit is refused before it reaches you — you will not see a signing prompt for it.                      | Eine Anfrage über dem Limit wird abgelehnt, bevor sie bei Ihnen ankommt — dafür erscheint keine Signatur-Abfrage.                               |

Rules the copy follows: no word for a state the wallet cannot prove ("safe", "trusted",
"official"); the domain is never described as "verified" without the sentence that says
_only_ the domain is; the word "website" rather than "dApp" wherever a user reads it, and
`dApp` only in headings that name the feature.

## What the screens cannot do yet

Reported rather than faked. Each needs a plugin command that the transport does not expose:

- **Wallet-initiated key rotation** (TASK-084 AC #3). `xchonnect-core` has
  `Session::begin_rotation`, and the session loop accepts a dApp's offer, but there is no
  command for the user to start one. The screen shows the key epoch and that rotation
  happens automatically; a `rotate_session_keys` command would make the action real.
- **Return to the dApp** (TASK-085 AC #1, #2). The verified `return_url` reaches the plugin
  as `VerifiedUri::return_url` and the core already refuses one that leaves the dApp's
  domain, but it is neither persisted with the session nor surfaced on `SessionSummary`.
  The guard is implemented and tested in `xchonnectLink.ts`; it has nothing to guard until
  the field exists.
- **Same-device requests at all** (TASK-085 AC #1). The pairing reply sets `WalletMeta`
  with a name only. Without `link: "https://clapandpay.com"` a dApp cannot build a `/req`
  link and fails with `no_wallet_link`.
- **OS link delivery** (TASK-072 AC #2). `tauri-plugin-deep-link` is not installed, so
  nothing hands the app a `/pair` or `/req` URL. `handleIncomingXchonnectLink()` is the
  single entry point that a deep-link listener calls; `docs/xchonnect-links.md` has the
  wiring and the hosting steps.
