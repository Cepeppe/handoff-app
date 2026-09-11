# GitHub Copilot

Baton funziona con GitHub Copilot oltre che con Claude Code, Codex, Cursor e OpenCode: nella chat
di VS Code, dove gira Copilot, e nella Copilot CLI, `copilot`. Un handoff che parte da Copilot va
allo stesso modo: il pannello si apre, fai i passi uno alla volta, e Copilot viene a sapere com'è
andata. Copilot ha un supporto **base** dove Claude Code ha quello *completo*, e questa pagina
spiega che cosa cambia per te.

## Registrarlo

Baton trova Copilot quando esiste la cartella della Copilot CLI, `%USERPROFILE%\.copilot`, o
quella delle impostazioni di VS Code, `%APPDATA%\Code\User`, oppure quando `copilot` o `code` è
nel tuo `PATH`. Si offre di registrarsi al primo avvio, o più tardi da Impostazioni → Agenti →
**Registra**, e la schermata di consenso mostra **due** modifiche, una per ogni posto in cui gira
Copilot:

- la voce del server MCP per la Copilot CLI, in `%USERPROFILE%\.copilot\mcp-config.json`;
- la voce del server MCP per VS Code, in `%APPDATA%\Code\User\mcp.json`.

[La schermata di consenso](../consent-screen.md#le-modifiche-per-github-copilot) mostra
esattamente che cosa viene scritto e perché. Se hai impostato `COPILOT_HOME`, la CLI tiene il suo
file in quella cartella, ed è lì che Baton lo scrive.

VS Code avvia il server di Baton la prima volta che una chat ne ha bisogno, e ti chiede se ti fidi
di lui la prima volta che parte: rispondi di sì. La Copilot CLI legge il suo file quando parte una
sessione: riavvia le sessioni che erano già in corso.

**Un file con dei commenti non viene modificato.** Riscrivere un file così li perderebbe. Se uno
dei due file ne ha, Baton lo dice e non registra niente; togli i commenti, oppure scrivi le voci a
mano come le mostra la schermata di consenso.

## Che cosa funziona come con Claude Code

- **Tutto l'handoff**: i passi, **Chiedi**, **Nota**, **Salta**, **Rimanda**, **Abbandona**, la
  verifica, e il runbook scritto dopo un handoff verificato.
- **Gli screenshot come immagini.** La Copilot CLI passa al modello un'immagine che arriva da
  Baton. La chat di VS Code non è stata misurata; se lì un'immagine non sembra arrivare,
  nell'anteprima preferisci **Invia testo**.
- **Le tue sessioni nel pannello.** Una scheda aperta da VS Code dice *GitHub Copilot*, poi il
  nome della cartella della finestra — la prima, in un workspace che ne ha più d'una. Tutte le
  chat di una finestra condividono quella sessione, e due finestre sono due sessioni. Una sessione
  della Copilot CLI porta il nome della cartella in cui è partita.
- **Le tue richieste arrivano alla finestra giusta.** Quando apri una richiesta con la
  scorciatoia, Baton la copia e porta in primo piano la finestra di VS Code il cui titolo nomina
  la cartella di quella sessione.

## Che cosa cambia

### Nessuno lo ricorda all'agente

Claude Code esegue un hook alla fine di ogni turno, e l'hook di Baton ricorda all'agente, una
volta, ciò che lo sta aspettando. Copilot ha degli hook suoi in tutti e due i posti, ma **nessuno
risponde all'hook di Baton in un modo di cui l'agente tenga conto**, quindi Baton non ne registra
nessuno. La Copilot CLI esegue anche, come suoi, gli hook di Claude Code nel
`.claude\settings.json` di un progetto: se Baton è registrato per Claude Code in quel progetto, un
turno di Copilot lì arriva a Baton, e Baton gli risponde con niente invece che con i promemoria di
un altro agente. In pratica:

- **Un passo che rimandi** torna all'agente con l'istruzione di riprenderlo prima di chiudere il
  turno, e di tenere l'id dell'handoff nei suoi appunti, perché nessuno glielo ricorderà. Se non
  lo fa, l'handoff aspetta nel pannello, e **Riprendi** lì lo rimette in corso e copia una frase
  da incollare in Copilot.
- **Una richiesta che apri** arriva a Copilot solo attraverso gli appunti: Baton copia la frase e
  porta la finestra in primo piano, e tu la incolli in una chat. Una richiesta aperta mentre non
  c'è nessuna sessione di Copilot aspetta, e viene copiata di nuovo quando parte la prima
  sessione.
- **Una risposta che l'agente non ha ancora raccolto** aspetta in Baton finché l'agente non
  chiama di nuovo.

### Quanto dura una chiamata

La voce della Copilot CLI alza il suo limite, solo per il server di Baton, a 30 minuti, come per
Claude Code. La voce di VS Code non ha un'impostazione così; lì, dopo cinquanta secondi il server
di Baton dice all'agente che l'handoff è ancora in corso, e l'agente lo riprende subito. Non si
perde nulla in nessuno dei due casi, e non c'è niente da impostare.

### Copilot chiede prima di una chiamata

Le voci di Baton non concedono niente oltre sé stesse. VS Code ti chiede il permesso prima che una
chat usi uno degli strumenti di Baton, come fa per qualunque server MCP, e lo stesso fa la Copilot
CLI in una sessione interattiva. Nella modalità di stampa, `copilot -p`, non c'è nessuno a cui
chiedere; permetti gli strumenti di Baton sulla sua riga di comando con `--allow-tool=handoff`,
che permette quelli e nient'altro. Baton non lo concede al posto tuo.

## Controllare la registrazione

```text
copilot mcp list
```

elenca i server che legge la Copilot CLI, e tra questi c'è `handoff` una volta che Baton è
registrato; `copilot mcp get handoff` ne mostra la voce. In VS Code, il comando **MCP: List
Servers** mostra `handoff` tra i server delle tue impostazioni utente.

## Un progetto invece di questo utente

In Impostazioni → Agenti → **Dove** → **Un progetto**, Baton scrive la voce della CLI in
`.github\mcp.json` e quella di VS Code in `.vscode\mcp.json` dentro la cartella che scegli — non in
`.mcp.json`, che è di Claude Code. La Copilot CLI legge i file di un progetto solo in una cartella
di cui le hai detto di fidarsi, e VS Code ti chiede di fidarti di un server di un file del
workspace prima del suo primo avvio. Baton non si fida di nessuno dei due al posto tuo.

## Tornare indietro

Impostazioni → Agenti → **Rimuovi** cancella la voce `"handoff"` di Baton da ciascun file,
riconosciuta dal percorso del server di Baton nel suo `command`, e nient'altro: gli altri server
restano com'erano, nello stesso ordine. Se la voce di Baton era l'unica dentro `"mcpServers"` o
`"servers"`, anche la chiave rimasta vuota viene tolta. Come per ogni modifica, prima viene
salvata una copia di ciascun file accanto.
