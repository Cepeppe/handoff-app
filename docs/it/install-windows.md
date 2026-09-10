# Installare Baton su Windows

Baton è costruito e provato su Windows 11 a 64 bit. Si installa solo per il tuo utente di
Windows e non richiede diritti di amministratore.

## Che cosa serve

- **Il programma di installazione.** Una release si chiama
  `Baton-<versione>-win32-x64-setup.exe`; quello che costruisci tu dai sorgenti si chiama
  `Baton_<versione>_x64-setup.exe`.
- **Microsoft Edge WebView2**, il componente di Windows che disegna le finestre fatte come
  pagine web. Windows 11 lo include. Se manca, il programma di installazione scarica
  l'installer di Microsoft da `go.microsoft.com`: è l'unica volta in cui il programma di
  installazione usa la rete.
- **Un agente supportato**: Claude Code. Baton lo cerca al primo avvio.

## Controlla il file

Se la release comprende un file `SHA256SUMS`, confronta il programma di installazione con
quello prima di eseguirlo. In PowerShell, nella cartella in cui l'hai salvato:

```powershell
Get-FileHash .\Baton-<versione>-win32-x64-setup.exe -Algorithm SHA256
```

L'hash stampato deve essere quello scritto accanto al nome del file in `SHA256SUMS`. Se non
lo è, non eseguire il programma di installazione.

## L'avviso di SmartScreen

Questa versione di Baton non è ancora firmata con un certificato di firma del codice, quindi
Windows non può sapere chi l'ha pubblicata. Quando apri un programma di installazione
scaricato da internet, Microsoft Defender SmartScreen lo ferma con una finestra blu:

> **PC protetto da Windows**
>
> Microsoft Defender SmartScreen ha impedito l'avvio di un'app non riconosciuta.
> L'esecuzione di questa app potrebbe costituire un rischio per il PC.

Per installare comunque Baton:

1. Fai clic su **Ulteriori informazioni**. La finestra mostra il nome del file e **Autore:
   Autore sconosciuto**.
2. Controlla che il nome del file sia il programma di installazione che hai scaricato, dal
   posto che ti aspettavi, e che l'hash corrispondesse (sopra).
3. Fai clic su **Esegui comunque**.

L'avviso riguarda la firma che manca, non qualcosa che il programma di installazione fa. Una
versione successiva sarà firmata, e l'avviso sparirà con lei.

Due casi sono diversi:

- **Smart App Control è attivo** (Sicurezza di Windows → Controllo delle app e del browser →
  Smart App Control). Allora Windows giudica da sé ogni programma, e la finestra qui sopra non
  compare. Può eseguire il programma di installazione senza nessun avviso, oppure bloccarlo
  con una notifica che non offre **Esegui comunque**; non ha eccezioni per un singolo
  programma, quindi dove blocca il programma di installazione Baton non si può installare su
  quel PC finché non sarà firmato. Nemmeno il programma di disinstallazione è firmato, e può
  essere bloccato allo stesso modo: vale la pena avviarlo una seconda volta, perché sul PC su
  cui Baton è costruito lo stesso programma di disinstallazione è stato bloccato una volta ed
  è partito la seconda. Non disattivare Smart App Control per un programma: Windows lo
  riattiva solo reimpostando il PC.
- **Il PC è gestito dalla tua organizzazione** e non consente programmi non firmati: la
  finestra non offre **Esegui comunque**.

Un programma di installazione costruito sullo stesso computer di solito non mostra nessun
avviso di SmartScreen, perché il file non è stato scaricato.

## Installare

Esegui il programma di installazione e seguilo. Installa solo per il tuo utente — senza
richiesta di amministratore — in `%LOCALAPPDATA%\Baton\`:

| File | Che cos'è |
|---|---|
| `handoff-app.exe` | Baton stesso: il pannello, l'icona nella barra, lo storico locale |
| `handoff-mcp.exe` | il server MCP che il tuo agente avvia; Baton scrive il suo percorso nelle impostazioni dell'agente |
| `patterns\` | l'elenco pubblico dei formati di segreti che Baton nasconde in automatico |
| `models\ocrs\` | i modelli di riconoscimento del testo inclusi, con la loro licenza |

Aggiunge un collegamento nel menu Start, e uno sul desktop se lo chiedi. Niente viene
eseguito come servizio e niente viene installato per altri utenti.

## Il primo avvio

Baton apre un breve benvenuto:

1. **Benvenuto in Baton.**
2. **I tuoi agenti**: gli agenti che Baton ha trovato, e le modifiche esatte che farebbe
   alle loro impostazioni. Non viene scritto nulla finché non premi **Accetta e registra**.
   [La schermata di consenso](consent-screen.md) spiega ogni riga.
3. **Avvio con la sessione**: la casella *Avvia Baton quando accedo* è spuntata. Così Baton
   parte nascosto nella barra quando accedi a Windows. Togli la spunta se preferisci avviarlo
   tu; finché non è in esecuzione, un handoff avviene nella chat dell'agente.
4. **La tua scorciatoia**: `Ctrl+Alt+H` dice a Baton che cosa stai per fare, da qualunque
   punto (vedi [Usare il pannello](using-the-overlay.md)).
5. **Pronto.** Da qui in poi Baton vive nella barra.

Poi riavvia le sessioni dei tuoi agenti: Claude Code legge le sue impostazioni quando una
sessione parte.

## Dove Baton tiene le sue cose

| Cartella | Che cosa contiene | Rimossa dalla disinstallazione |
|---|---|---|
| `%LOCALAPPDATA%\Baton\` | il programma | sempre |
| `%APPDATA%\Baton\` | lo storico (`handoff.sqlite`) e i rapporti di crash (`crashes\`) | solo se spunti *Delete the application data* |
| `%APPDATA%\com.cepeppe.baton\`, `%LOCALAPPDATA%\com.cepeppe.baton\` | i dati della finestra di Baton: cache e archivio di WebView2 | solo se spunti *Delete the application data* |
| `%USERPROFILE%\.handoff\` | il token del canale e i tuoi runbook, condivisi con il server MCP | mai |
| `%USERPROFILE%\.claude.json`, `%USERPROFILE%\.claude\settings.json` | le impostazioni del tuo agente, dove Baton ha aggiunto le sue righe con il tuo consenso | mai: usa prima **Rimuovi** (sotto) |

## Aggiornare

Questa versione non controlla gli aggiornamenti (lo dice Impostazioni → Aggiornamenti). Per
aggiornare, esegui il programma di installazione della nuova versione sopra quella
installata; se chiede se disinstallare prima la versione installata, **Do not uninstall**
sostituisce i file sul posto. Il programma di installazione chiude Baton se è in esecuzione:
dopo, riavvialo tu se non l'ha fatto lui.

Le impostazioni dei tuoi agenti non cambiano: puntano a
`%LOCALAPPDATA%\Baton\handoff-mcp.exe`, e quel percorso resta lo stesso. Le sessioni degli
agenti già avviate continuano a usare il server con cui sono partite finché non le riavvii:
Windows non può sovrascrivere un programma in uso, quindi il programma di installazione
rinomina quel file in `handoff-mcp.<versione precedente>.old.exe` e mette il nuovo server al
solito percorso, dove lo trova ogni sessione che avvii da lì in poi. Baton cancella il
vecchio file la prima volta che si avvia dopo che quelle sessioni sono finite.

## Disinstallare

1. **Per prima cosa togli Baton dai tuoi agenti.** In Baton apri Impostazioni → Agenti e
   premi **Rimuovi** per ogni agente registrato. Cancella esattamente le righe che Baton ha
   aggiunto e nient'altro (vedi [La schermata di consenso](consent-screen.md)).
2. Chiudi le sessioni dei tuoi agenti, poi esci da Baton dalla sua icona nella barra:
   **Esci**. Una sessione ancora in corso tiene aperto il file del suo server, Windows non può
   cancellare un programma in uso, e il file resterebbe in `%LOCALAPPDATA%\Baton\`.
3. Disinstallalo da Impostazioni di Windows → App → App installate → Baton → **Disinstalla**.

La disinstallazione rimuove il programma, compreso ogni `handoff-mcp.<versione>.old.exe`
lasciato da un aggiornamento, i collegamenti e l'avvio all'accesso. Il programma di
disinstallazione (in inglese) chiede se eseguire **Delete the application data**, senza
spunta: spuntala per rimuovere tutto quello che Baton ha scritto da sé — `%APPDATA%\Baton\`
(lo storico e i rapporti di crash) e le due cartelle `com.cepeppe.baton`. Non rimuove mai
`%USERPROFILE%\.handoff\`, perché lì ci sono i tuoi runbook e il server MCP usa anche lui
quella cartella, e non modifica mai le impostazioni dei tuoi agenti.

### Se hai disinstallato senza premere Rimuovi

Le impostazioni del tuo agente puntano ancora a un server che non c'è più: Claude Code
segnala che il server MCP `handoff` non è riuscito a partire, e un hook fallisce alla fine di
ogni turno. Togli a mano le righe di Baton:

- in `%USERPROFILE%\.claude.json`, cancella la voce `"handoff"` dentro `"mcpServers"`;
- in `%USERPROFILE%\.claude\settings.json`, dentro `"hooks"` → `"Stop"` e `"SubagentStop"`,
  cancella le voci il cui `command` finisce con `handoff-mcp.exe" hook stop`. Lascia com'è
  ogni altra voce.

Alla fine controlla che ogni file sia ancora JSON valido. Ogni volta che Baton ha modificato
uno di questi file, prima ne ha salvato una copia accanto, chiamata
`<file>.handoff-backup-<data e ora>`; ripristinarne una annulla anche ogni modifica fatta al
file dopo di allora.
