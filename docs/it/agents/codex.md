# Codex CLI

Baton funziona con Codex CLI oltre che con Claude Code. Un handoff che parte da Codex va allo
stesso modo: il pannello si apre, fai i passi uno alla volta, e Codex viene a sapere com'è
andata. Codex ha un supporto **base** dove Claude Code ha quello *completo*, e questa pagina
spiega che cosa cambia per te.

## Registrarlo

Baton trova Codex quando `codex` è nel tuo `PATH`, oppure quando esiste la sua cartella delle
impostazioni, `%USERPROFILE%\.codex`. Si offre di registrarsi lì al primo avvio, o più tardi da
Impostazioni → Agenti → **Registra**, e la schermata di consenso mostra **una** sola modifica:
la voce del server MCP nel `config.toml` di Codex. [La schermata di consenso](../consent-screen.md#la-modifica-per-codex)
mostra esattamente che cosa viene scritto e perché, compresa l'approvazione che Codex riceve
per gli strumenti di Baton.

Se hai impostato `CODEX_HOME`, Codex tiene lì le sue impostazioni, e Baton scrive lì. Codex
legge le impostazioni quando una sessione parte, quindi riavvia le sessioni che erano già
aperte.

## Che cosa funziona come con Claude Code

- **Tutto l'handoff**: i passi, **Chiedi**, **Nota**, **Salta**, **Rimanda**, **Abbandona**, la
  verifica, e il runbook scritto dopo un handoff verificato.
- **Gli screenshot come immagini.** Codex passa al suo modello un'immagine che arriva da Baton,
  quindi l'anteprima offre **Invia immagine** oltre a **Invia testo**.
- **Un'attesa lunga.** Baton dà 30 minuti alle chiamate di Codex al proprio server. Se un
  handoff dura di più non si perde nulla: un minuto prima del limite all'agente viene detto che
  l'handoff è ancora in corso, e lo riprende.
- **Le tue sessioni nel pannello.** Una scheda aperta da Codex lo dice: *Codex CLI*, poi il nome
  della cartella del progetto.

## Che cosa cambia: nessuno lo ricorda all'agente

Claude Code esegue un hook alla fine di ogni turno, e l'hook di Baton ricorda all'agente, una
volta, ciò che lo sta aspettando. **Codex non esegue un hook del genere**, quindi con Codex
nessuno chiede niente alla fine di un turno. In pratica:

- **Un passo che rimandi** torna all'agente con l'istruzione di riprenderlo prima di chiudere il
  turno, e di tenere l'id dell'handoff nei suoi appunti, perché nessuno glielo ricorderà. Un
  agente attento lo fa. Se non lo fa, l'handoff aspetta nel pannello, e **Riprendi** lì lo
  rimette in corso e copia una frase da incollare in Codex.
- **Una richiesta che apri** con la scorciatoia arriva a Codex solo attraverso gli appunti:
  Baton copia la frase e porta il terminale in primo piano, e tu la incolli. Una richiesta
  aperta mentre non c'è nessuna sessione di Codex aspetta, e viene copiata di nuovo quando parte
  la prima sessione.
- **Una risposta che l'agente non ha ancora raccolto** aspetta in Baton finché l'agente non
  chiama di nuovo.

È ciò che Codex permette oggi, non un'impostazione di Baton. Se una versione futura di Codex
eseguirà gli hook per sessioni come queste, il suo supporto potrà diventare completo.

## Controllare la registrazione

```text
codex mcp get handoff
```

stampa la voce che Codex legge. Indica il server di Baton come comando e mostra
`tool_timeout_sec: 1800` e `default_tools_approval_mode: approve`.

## Un progetto invece di questo utente

In Impostazioni → Agenti → **Dove** → **Un progetto**, Baton scrive la stessa voce in
`.codex\config.toml` dentro la cartella che scegli. **Codex legge quel file solo per un progetto
che hai segnato come attendibile in Codex**; fino ad allora la voce viene ignorata. Baton non
segna mai un progetto come attendibile al posto tuo.

## Tornare indietro

Impostazioni → Agenti → **Rimuovi** cancella la sezione `[mcp_servers.handoff]` dal
`config.toml` di Codex, riconosciuta dal percorso del server di Baton nel suo `command`, e
nient'altro: commenti, altri server e profili restano esattamente com'erano. Come per ogni
modifica, prima viene salvata una copia del file accanto.
