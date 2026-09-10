# Risoluzione dei problemi

## Per prima cosa: `doctor`

Il server MCP che arriva con Baton sa controllare tutta la catena. In PowerShell:

```powershell
& "$env:LOCALAPPDATA\Baton\handoff-mcp.exe" doctor
```

Stampa che cosa ha determinato il server (la sua versione, l'agente, il timeout degli
strumenti), se il token del canale si può leggere, se Baton risponde sul suo canale e se la
cartella dei runbook si può leggere. Finisce con `doctor: nothing to repair`, oppure con una
riga `problem:` per ogni cosa da sistemare. Il token non viene mai stampato, quindi il rapporto
si può condividere. Lanciato fuori da un agente, mostra l'agente come `unknown`: è normale.

La documentazione del server spiega ogni riga del rapporto
([`doctor`](https://github.com/Cepeppe/handoff-mcp/blob/main/docs/install-without-app.md)).

## Sintomi

### Gli handoff compaiono nella chat invece che nel pannello

È la modalità testo: il server non ha trovato Baton quando l'agente ha aperto l'handoff.

- Baton non è in esecuzione: avvialo. Una sessione già aperta lo trova entro circa 30
  secondi, e il prossimo handoff usa il pannello.
- Baton è in esecuzione: lancia `doctor` e leggi le sue righe `problem:`.

### L'agente dice che non riesce a raggiungere Baton, o che il canale l'ha rifiutato

Il token che il server e Baton condividono non corrisponde, per esempio dopo aver ripristinato
un backup di `%USERPROFILE%\.handoff\`. In Impostazioni → Agenti premi **Ripara il token**, poi
riavvia la sessione dell'agente (o ricollega il server, sotto).

### L'agente dice che Baton va aggiornato

Il server e Baton parlano versioni diverse del loro canale: l'agente sta avviando un server che
non è arrivato con questo Baton. In Impostazioni → Agenti, **Ripara** fa puntare di nuovo
l'agente al server che arriva con Baton.

### In Claude Code non ci sono gli strumenti degli handoff

- Impostazioni → Agenti dovrebbe dire **Registrato**. Se no, **Registra** o **Ripara**.
- Riavvia la sessione di Claude Code: legge le sue impostazioni quando parte.
- In Claude Code, `/mcp` elenca i server; `handoff` dovrebbe risultare collegato.

### Il server si è fermato a metà di una sessione

Claude Code mostra il server `handoff` come fallito. L'handoff è al sicuro in Baton. In Claude
Code lancia `/mcp` e ricollega `handoff`; l'agente riprende l'handoff da dov'era.

### *Baton è stato spostato. Ripara la registrazione perché i tuoi agenti lo ritrovino.*

Baton ora si trova in un punto diverso dal percorso scritto nelle impostazioni dei tuoi agenti.
Impostazioni → Agenti → **Ripara** mostra la modifica e la riscrive. Fino ad allora gli handoff
avvengono nella chat.

### La scorciatoia non fa niente

Un altro programma tiene `Ctrl+Alt+H`. Scegli un'altra combinazione in Impostazioni → Generale
→ Scorciatoia, oppure usa **Nuova richiesta** nel menu della barra.

### Il terminale non viene in primo piano dopo una richiesta

Alcuni terminali, tra cui quello di VS Code, non permettono a Baton di trovare o alzare la
finestra giusta. Incolla tu la richiesta dagli appunti; se te ne dimentichi, all'agente viene
ricordata alla fine del suo turno.

### Il testo degli screenshot viene letto male

Windows legge il testo solo nelle lingue di cui è installata la funzionalità di riconoscimento
ottico dei caratteri; altrimenti lo legge il motore incluso in Baton, che legge meglio
l'inglese. Aggiungi la lingua in Impostazioni di Windows → Data/ora e lingua → Lingua e area
geografica, con le sue funzionalità facoltative.

### *Non è stato possibile leggere la cattura, quindi non è stato nascosto niente in automatico.*

Non è partito nessun motore di riconoscimento del testo, il che vuol dire che nell'installazione
mancano i modelli inclusi. Reinstalla Baton. Nel frattempo nascondi a mano quello che serve
prima di inviare.

### *Sessione staccata; l'esito arriverà al prossimo resume*

La sessione dell'agente è finita mentre l'handoff era aperto. Se vuoi continua pure; quello che
fai viene consegnato alla prossima sessione che riprende l'handoff. Chiedi a una qualunque
sessione dell'agente di riprenderlo con il suo id (`hf_…`, **Copia l'id** su una scheda
rimasta senza esito raccolto).

### *Di quale sessione si tratta?*

Due sessioni lavorano nella stessa cartella e Baton non è riuscito a capire da quale delle due
è arrivato un hook. Scegli quella in cui stai lavorando.

### L'avviso di SmartScreen durante l'installazione

Vedi [Installare su Windows](install-windows.md).

## Che cosa condividere quando chiedi aiuto

- l'uscita di `doctor` — non contiene mai il token;
- il rapporto di crash, se c'è (vedi [Rapporti di crash](crash-reports.md)) — prima leggilo;
- un'esportazione dello storico solo se serve, e dopo averla letta: contiene gli handoff stessi
  (vedi [Storico ed esportazione](log-and-export.md)).
