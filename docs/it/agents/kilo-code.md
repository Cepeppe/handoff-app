# Kilo Code

Baton funziona con Kilo Code oltre che con Claude Code, Codex, OpenCode, Cursor e GitHub
Copilot, su tutte e due le superfici di Kilo Code: la Kilo CLI (`kilo`) e l'estensione Kilo Code
per VS Code, che eseguono lo stesso programma. Un handoff che parte dall'una o dall'altra va allo
stesso modo: il pannello si apre, fai i passi uno alla volta, e Kilo viene a sapere com'è
andata. Kilo Code ha un supporto **base** dove Claude Code ha quello *completo*, e questa pagina
spiega che cosa cambia per te.

## Registrarlo

Baton trova Kilo Code quando `kilo` è nel tuo `PATH`, quando esiste la sua cartella delle
impostazioni, `%USERPROFILE%\.config\kilo`, oppure quando l'estensione Kilo Code è installata in
VS Code. Si offre di registrarsi lì al primo avvio, o più tardi da Impostazioni → Agenti →
**Registra**, e la schermata di consenso mostra **una** sola modifica: la voce del server MCP in
`kilo.json`, che la CLI e l'estensione leggono tutte e due.
[La schermata di consenso](../consent-screen.md#la-modifica-per-kilo-code) mostra esattamente
che cosa viene scritto e perché.

Baton scrive `%USERPROFILE%\.config\kilo\kilo.json`, oppure lo stesso file sotto
`XDG_CONFIG_HOME` se hai impostato quella variabile, che è dove guarda Kilo stesso. L'estensione
tiene impostazioni sue in `kilo.jsonc`, nella stessa cartella; Baton lascia stare quel file e
mette la sua voce in `kilo.json` accanto: Kilo li legge tutti e due. Kilo legge le impostazioni
quando una sessione parte — l'estensione quando il pannello di Kilo si apre per la prima volta
in una finestra — quindi riavvia le sessioni che erano già aperte, e ricarica le finestre di VS
Code in cui Kilo era aperto.

Kilo mette in ordine il file delle impostazioni quando lo legge: aggiunge in cima una riga
`"$schema"` e rientra con due spazi. Lo fa Kilo, e non cambia niente di ciò che ha scritto Baton.

**Un file con dei commenti non viene modificato.** Kilo ammette i commenti in `kilo.json`, e
riscrivere un file così li perderebbe. Se il tuo ne ha, Baton lo dice e non registra niente;
togli i commenti, oppure scrivi la voce a mano come la mostra la schermata di consenso.

## Che cosa funziona come con Claude Code

- **Tutto l'handoff**: i passi, **Chiedi**, **Nota**, **Salta**, **Rimanda**, **Abbandona**, la
  verifica, e il runbook scritto dopo un handoff verificato.
- **Gli screenshot come immagini**, se il tuo modello legge le immagini. Kilo passa un'immagine
  che arriva da Baton al modello che hai scelto, di qualunque fornitore sia. Un modello che legge
  solo testo riceve ogni parola della risposta ma non l'immagine; con uno di questi — tra cui il
  modello automatico gratuito di Kilo — nell'anteprima preferisci **Invia testo**.
- **Un'attesa lunga.** Baton dà 30 minuti alle chiamate di Kilo al proprio server, e con Kilo
  conta più che con la maggior parte degli altri: senza, Kilo rinuncia a una chiamata dopo un
  minuto. Se un handoff dura più di 30 minuti non si perde nulla: un minuto prima del limite
  all'agente viene detto che l'handoff è ancora in corso, e lo riprende.
- **Nessuna domanda prima di ogni chiamata.** Kilo usa gli strumenti di Baton senza chiedertelo,
  nella CLI come in VS Code, quindi la voce non concede niente oltre sé stessa.
- **Le tue sessioni nel pannello.** Una scheda aperta da Kilo lo dice: *Kilo Code*, poi il nome
  della cartella del progetto — per l'estensione, la cartella aperta in quella finestra di VS
  Code. Due finestre di VS Code sono due sessioni.

## Che cosa cambia: nessuno lo ricorda all'agente

Claude Code esegue un hook alla fine di ogni turno, e l'hook di Baton ricorda all'agente, una
volta, ciò che lo sta aspettando. **Kilo Code non ha un hook di quel tipo** — ha dei plugin, che
girano dentro Kilo — quindi con Kilo nessuno chiede niente alla fine di un turno. In pratica:

- **Un passo che rimandi** torna all'agente con l'istruzione di riprenderlo prima di chiudere il
  turno, e di tenere l'id dell'handoff nei suoi appunti, perché nessuno glielo ricorderà. Un
  agente attento lo fa. Se non lo fa, l'handoff aspetta nel pannello, e **Riprendi** lì lo
  rimette in corso e copia una frase da incollare in Kilo.
- **Una richiesta che apri** con la scorciatoia arriva a Kilo solo attraverso gli appunti: Baton
  copia la frase e porta in primo piano la finestra in cui gira la sessione — il terminale della
  CLI, oppure la finestra di VS Code dell'estensione — e tu la incolli in Kilo. Una richiesta
  aperta mentre non c'è nessuna sessione di Kilo aspetta, e viene copiata di nuovo quando parte
  la prima sessione.
- **Una risposta che l'agente non ha ancora raccolto** aspetta in Baton finché l'agente non
  chiama di nuovo.

È ciò che Kilo permette oggi, non un'impostazione di Baton.

## Controllare la registrazione

```text
kilo mcp list
```

elenca i server che Kilo legge, e tra questi c'è `handoff` una volta che Baton è registrato.
`kilo debug config` stampa la voce intera: il server di Baton come `command`, `HANDOFF_AGENT`
impostato a `kilo-code`, e `"timeout": 1800000`.

## Un handoff in VS Code, a mano

L'estensione non si può guidare con uno script, quindi ecco come vedere Kilo Code al lavoro in
VS Code, in circa cinque minuti:

1. In Baton, apri Impostazioni → Agenti, premi **Registra** accanto a Kilo Code, e accetta
   l'unica modifica.
2. In VS Code, ricarica ogni finestra in cui Kilo era aperto (Riquadro comandi → *Developer:
   Reload Window*).
3. Apri la cartella di un progetto, apri il pannello di Kilo, e avvia un compito che ha bisogno
   di una persona — per esempio: *Chiedimi, attraverso Baton, di creare un endpoint webhook nella
   dashboard di Stripe, poi verificalo.*
4. Il pannello apre una scheda che dice **Kilo Code · il nome di quella cartella**. Fai i passi,
   premi **Fatto**, e lascia che l'agente verifichi: la scheda finisce *verificato*.
5. Premi `Ctrl+Alt+H`, scrivi una richiesta e premi Invio: la finestra di VS Code viene in primo
   piano, e la frase è negli appunti. Incollala nella chat di Kilo: l'agente risponde con un
   handoff, che prende il posto della scheda della richiesta.
6. Tornato in Impostazioni → Agenti, premi **Rimuovi** accanto a Kilo Code, poi confronta
   `kilo.json` con la copia `kilo.json.handoff-backup-…` accanto: manca solo la voce
   `"handoff"`.

## Un progetto invece di questo utente

In Impostazioni → Agenti → **Dove** → **Un progetto**, Baton scrive la stessa voce in
`kilo.json` in cima alla cartella che scegli. Kilo la legge, sopra alle tue impostazioni, quando
lavora in quella cartella — la CLI avviata lì, oppure una finestra di VS Code con quella cartella
aperta — e non chiede prima se fidarsene.

## Tornare indietro

Impostazioni → Agenti → **Rimuovi** cancella la voce `"handoff"` da `kilo.json`, riconosciuta dal
percorso del server di Baton nel suo `command`, e nient'altro: gli altri server e le altre
impostazioni restano com'erano, nello stesso ordine, e `kilo.jsonc` non viene mai toccato. Se la
voce di Baton era l'unica dentro `"mcp"`, anche la chiave rimasta vuota viene tolta. Come per
ogni modifica, prima viene salvata una copia del file accanto.
