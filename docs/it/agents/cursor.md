# Cursor

Baton funziona con Cursor oltre che con Claude Code, Codex e OpenCode: nell'editor di Cursor e
nella sua Agent CLI, `agent`. Un handoff che parte da Cursor va allo stesso modo: il pannello si
apre, fai i passi uno alla volta, e Cursor viene a sapere com'è andata. Cursor ha un supporto
**base** dove Claude Code ha quello *completo*, e questa pagina spiega che cosa cambia per te.

## Registrarlo

Baton trova Cursor quando esiste la sua cartella delle impostazioni, `%USERPROFILE%\.cursor` —
la creano sia l'editor sia la Agent CLI la prima volta che partono — oppure quando `cursor` o
`cursor-agent` è nel tuo `PATH`. Si offre di registrarsi lì al primo avvio, o più tardi da
Impostazioni → Agenti → **Registra**, e la schermata di consenso mostra **una** sola modifica:
la voce del server MCP in `mcp.json`.
[La schermata di consenso](../consent-screen.md#la-modifica-per-cursor) mostra esattamente che
cosa viene scritto e perché.

Baton scrive `%USERPROFILE%\.cursor\mcp.json`, il file che leggono sia l'editor sia la Agent
CLI, quindi una sola registrazione vale per tutti e due. L'editor avvia i server di quel file
quando si apre una finestra: chiudi e riapri le finestre di Cursor che erano già aperte, e
riavvia le sessioni della CLI che erano già in corso.

**Un file con dei commenti non viene modificato.** Riscrivere un file così li perderebbe. Se il
tuo ne ha, Baton lo dice e non registra niente; togli i commenti, oppure scrivi la voce a mano
come la mostra la schermata di consenso.

## Che cosa funziona come con Claude Code

- **Tutto l'handoff**: i passi, **Chiedi**, **Nota**, **Salta**, **Rimanda**, **Abbandona**, la
  verifica, e il runbook scritto dopo un handoff verificato.
- **Gli screenshot come immagini.** La Agent CLI passa al modello un'immagine che arriva da
  Baton. La chat dell'editor non è stata misurata; se lì un'immagine non sembra arrivare,
  nell'anteprima preferisci **Invia testo**.
- **Le tue sessioni nel pannello, una per finestra.** Una scheda aperta dall'editor di Cursor
  dice *Cursor*, poi il nome della cartella della finestra — la prima, in un workspace che ne ha
  più d'una. Tutte le chat di una finestra condividono quella sessione, e due finestre sono due
  sessioni. Una sessione della Agent CLI porta il nome della cartella in cui è partita.
- **Le tue richieste arrivano alla finestra giusta.** Quando apri una richiesta con la
  scorciatoia, Baton la copia e porta in primo piano la finestra di Cursor il cui titolo nomina
  la cartella di quella sessione.

## Che cosa cambia

### Nessuno lo ricorda all'agente

Claude Code esegue un hook alla fine di ogni turno, e l'hook di Baton ricorda all'agente, una
volta, ciò che lo sta aspettando. Cursor ha degli hook suoi, ma **nessuno di questi arriva a
Baton**: quello che Cursor passa a un hook non è quello che legge l'hook di Baton, quindi Baton
non ne registra nessuno. Se Baton è registrato anche per Claude Code, Cursor può eseguire
quell'hook alla fine dei propri turni; lì resta in silenzio, e non ferma mai un turno di Cursor.
In pratica:

- **Un passo che rimandi** torna all'agente con l'istruzione di riprenderlo prima di chiudere il
  turno, e di tenere l'id dell'handoff nei suoi appunti, perché nessuno glielo ricorderà. Se non
  lo fa, l'handoff aspetta nel pannello, e **Riprendi** lì lo rimette in corso e copia una frase
  da incollare in Cursor.
- **Una richiesta che apri** arriva a Cursor solo attraverso gli appunti: Baton copia la frase e
  porta la finestra in primo piano, e tu la incolli in una chat. Una richiesta aperta mentre non
  c'è nessuna sessione di Cursor aspetta, e viene copiata di nuovo quando parte la prima
  sessione.
- **Una risposta che l'agente non ha ancora raccolto** aspetta in Baton finché l'agente non
  chiama di nuovo.

### Una chiamata dura un minuto

Cursor dà a una chiamata di uno strumento un limite che nessuna voce può alzare: la Agent CLI
interrompe una chiamata dopo sessanta secondi, e l'editor aspetta di più. Così, mentre lavori a
un handoff, dopo cinquanta secondi il server di Baton dice all'agente che l'handoff è ancora in
corso, e l'agente lo riprende subito. Non si perde nulla, e non c'è niente da impostare.

### Cursor chiede prima di una chiamata

La voce di Baton non concede niente oltre sé stessa. L'editor ti chiede il permesso prima di
usare uno degli strumenti di Baton, come fa per qualunque server MCP. La modalità di stampa della
Agent CLI, `agent -p`, non ha nessuno a cui chiedere e rifiuta uno strumento così; per lasciarle
usare quelli di Baton, aggiungi `Mcp(handoff:*)` alla lista `allow` di `permissions` in
`.cursor\cli.json` nel progetto (oppure in `%USERPROFILE%\.cursor\cli-config.json`, per ogni
progetto). In un progetto che non ha ancora un `cli.json`, il file intero è:

```json
{ "permissions": { "allow": ["Mcp(handoff:*)"] } }
```

Permette gli strumenti di Baton e nient'altro. Baton non la scrive al posto tuo.

## Controllare la registrazione

```text
agent mcp list
```

elenca i server che la Agent CLI legge nella cartella da cui la lanci, e tra questi c'è
`handoff` una volta che Baton è registrato. Per l'editor li elencano le impostazioni di Cursor.

## Un progetto invece di questo utente

In Impostazioni → Agenti → **Dove** → **Un progetto**, Baton scrive la stessa voce in
`.cursor\mcp.json` dentro la cartella che scegli. Cursor carica un server di un file di progetto
solo dopo che l'hai approvato: l'editor lo chiede, e la Agent CLI lo rifiuta finché non esegui
`agent mcp enable handoff` in quella cartella. Baton non lo approva mai al posto tuo.

## Tornare indietro

Impostazioni → Agenti → **Rimuovi** cancella la voce `"handoff"` da `mcp.json`, riconosciuta dal
percorso del server di Baton nel suo `command`, e nient'altro: gli altri server restano com'erano,
nello stesso ordine. Se la voce di Baton era l'unica dentro `"mcpServers"`, anche la chiave
rimasta vuota viene tolta. Come per ogni modifica, prima viene salvata una copia del file
accanto.
