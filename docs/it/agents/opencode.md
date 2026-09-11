# OpenCode

Baton funziona con OpenCode oltre che con Claude Code e Codex. Un handoff che parte da OpenCode
va allo stesso modo: il pannello si apre, fai i passi uno alla volta, e OpenCode viene a sapere
com'è andata. OpenCode ha un supporto **base** dove Claude Code ha quello *completo*, e questa
pagina spiega che cosa cambia per te.

## Registrarlo

Baton trova OpenCode quando `opencode` è nel tuo `PATH`, oppure quando esiste la sua cartella
delle impostazioni, `%USERPROFILE%\.config\opencode`. Si offre di registrarsi lì al primo avvio,
o più tardi da Impostazioni → Agenti → **Registra**, e la schermata di consenso mostra **una**
sola modifica: la voce del server MCP in `opencode.json`.
[La schermata di consenso](../consent-screen.md#la-modifica-per-opencode) mostra esattamente
che cosa viene scritto e perché.

Baton scrive `%USERPROFILE%\.config\opencode\opencode.json`, oppure lo stesso file sotto
`XDG_CONFIG_HOME` se hai impostato quella variabile, che è dove guarda OpenCode stesso. Se tieni
le tue impostazioni in `opencode.jsonc`, Baton lascia stare quel file e mette la sua voce in
`opencode.json` accanto: OpenCode li legge tutti e due. OpenCode legge le impostazioni quando una
sessione parte, quindi riavvia le sessioni che erano già aperte.

**Un file con dei commenti non viene modificato.** OpenCode ammette i commenti in
`opencode.json`, e riscrivere un file così li perderebbe. Se il tuo ne ha, Baton lo dice e non
registra niente; togli i commenti, oppure scrivi la voce a mano come la mostra la schermata di
consenso.

## Che cosa funziona come con Claude Code

- **Tutto l'handoff**: i passi, **Chiedi**, **Nota**, **Salta**, **Rimanda**, **Abbandona**, la
  verifica, e il runbook scritto dopo un handoff verificato.
- **Gli screenshot come immagini**, se il tuo modello legge le immagini. OpenCode passa
  un'immagine che arriva da Baton al modello che hai scelto. Un modello che legge solo testo
  riceve ogni parola della risposta ma non l'immagine; con uno di questi, nell'anteprima
  preferisci **Invia testo**.
- **Un'attesa lunga.** Baton dà 30 minuti alle chiamate di OpenCode al proprio server, e con
  OpenCode conta più che con gli altri: senza, OpenCode rinuncia a una chiamata dopo un minuto.
  Se un handoff dura più di 30 minuti non si perde nulla: un minuto prima del limite all'agente
  viene detto che l'handoff è ancora in corso, e lo riprende.
- **Nessuna domanda prima di ogni chiamata.** OpenCode usa gli strumenti di Baton senza
  chiedertelo, quindi la voce non concede niente oltre sé stessa.
- **Le tue sessioni nel pannello.** Una scheda aperta da OpenCode lo dice: *OpenCode*, poi il
  nome della cartella del progetto.

## Che cosa cambia: nessuno lo ricorda all'agente

Claude Code esegue un hook alla fine di ogni turno, e l'hook di Baton ricorda all'agente, una
volta, ciò che lo sta aspettando. **OpenCode non ha un hook di quel tipo** — ha dei plugin, che
girano dentro OpenCode — quindi con OpenCode nessuno chiede niente alla fine di un turno. In
pratica:

- **Un passo che rimandi** torna all'agente con l'istruzione di riprenderlo prima di chiudere il
  turno, e di tenere l'id dell'handoff nei suoi appunti, perché nessuno glielo ricorderà. Un
  agente attento lo fa. Se non lo fa, l'handoff aspetta nel pannello, e **Riprendi** lì lo
  rimette in corso e copia una frase da incollare in OpenCode.
- **Una richiesta che apri** con la scorciatoia arriva a OpenCode solo attraverso gli appunti:
  Baton copia la frase e porta il terminale in primo piano, e tu la incolli. Una richiesta
  aperta mentre non c'è nessuna sessione di OpenCode aspetta, e viene copiata di nuovo quando
  parte la prima sessione.
- **Una risposta che l'agente non ha ancora raccolto** aspetta in Baton finché l'agente non
  chiama di nuovo.

È ciò che OpenCode permette oggi, non un'impostazione di Baton.

## Controllare la registrazione

```text
opencode mcp list
```

elenca i server che OpenCode legge, e tra questi c'è `handoff` una volta che Baton è registrato.
`opencode debug config` stampa la voce intera: il server di Baton come `command`,
`HANDOFF_AGENT` impostato a `opencode`, e `"timeout": 1800000`.

## Un progetto invece di questo utente

In Impostazioni → Agenti → **Dove** → **Un progetto**, Baton scrive la stessa voce in
`opencode.json` in cima alla cartella che scegli. OpenCode la legge, sopra alle tue
impostazioni, quando parte in quella cartella o in una più in basso.

## Tornare indietro

Impostazioni → Agenti → **Rimuovi** cancella la voce `"handoff"` da `opencode.json`,
riconosciuta dal percorso del server di Baton nel suo `command`, e nient'altro: gli altri server
e le altre impostazioni restano com'erano, nello stesso ordine. Se la voce di Baton era l'unica
dentro `"mcp"`, anche la chiave rimasta vuota viene tolta. Come per ogni modifica, prima viene
salvata una copia del file accanto.
