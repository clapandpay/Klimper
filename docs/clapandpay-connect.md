# Bringing a Green Envelope wallet into Klimper

"Use my Clap and Pay money" on Klimper's start screen (`src/pages/ClapAndPayConnect.tsx`) brings
the wallet from the Green Envelope page into Klimper. A passkey belongs to a website, so the
check happens on that page; it answers with one line of text that only this Klimper can read,
and Klimper imports the key with Sage's ordinary key import. The protocol and its limits: the
wiki's `architecture/passkey-handoff.md`.

## Against the live page

Nothing to set up. Klimper opens `https://envelope.clapandpay.com/connect?...`. The wallet's passkey
still belongs to `clapandpay.com`: the web wallet names that domain as its relying party on every
subdomain of it, which is why the same key comes back in Klimper.

## Against a local Green Envelope dev server

A wallet made on `pnpm dev` belongs to that address: the passkey's relying party is `localhost`
and a device wallet lives in that browser's storage for `http://localhost:5173`. The page Klimper
opens has to be exactly that origin, in the same browser.

1. In `green-envelope/`: `pnpm dev` (port 5173, testnet11).
2. In `klimper/.env.local` (git ignores it):

   ```sh
   VITE_CLAPANDPAY_ORIGIN=http://localhost:5173
   ```

   `localhost`, not `127.0.0.1`: a different host is a different origin and finds no wallet.

3. `pnpm tauri:dev` (restart it after changing `.env.local`; Vite reads it at start).
4. Klimper's start screen, **Use my Clap and Pay money**. The first button now says
   **Open localhost:5173**. It opens your **default browser**: that has to be the browser the
   wallet was made in (a passkey in iCloud Keychain is found from Safari and Chrome alike; a
   device wallet only in the browser that made it).
5. On the page: check that the six digits match, **Yes, it is me**, Touch ID, **Copy**. Keep
   the page open: it now shows a second number next to the answer.
6. Back in Klimper: **Paste what the page gave you**. Klimper shows the answer's number; tap
   **Same number** only if the page shows the same (an answer that did not come from the page,
   such as a swapped clipboard, cannot match it). The dev server is on testnet11; if Klimper is
   on mainnet it offers **Switch to testnet11 and bring it in**, which is the same switch as the
   network setting.

The app's one-time key travels in the link's fragment (`/connect#v=1&k=…`), which a browser
never sends to the server, and the page takes it out of the address bar and the history as soon
as it has read it. The first number is made from that public key: it catches the wrong page or a
mixed-up request, not an attacker who controls the device or the browser. The second number is
drawn from the secret the app and the page share, so an answer sealed by anyone else cannot show
it, and nobody can search for one that does.

The wallet appears as "Clap and Pay" 🟢, with the address the page shows. Remove the line from
`.env.local` to talk to clapandpay.com again.

## Proof

`pnpm e2e:handoff` in `green-envelope/` runs the whole round trip in headless Chrome: a virtual
platform authenticator with PRF makes a passkey wallet on a dev server, Klimper's own
`src/lib/clapandpayHandoff.ts` makes the request, the page approves it, and the answer must
decrypt to the key of the page's address and be refused by any other request. The Sage import
itself is Sage's `import_key` with the hex master key, unchanged.
