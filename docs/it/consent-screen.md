# La schermata di consenso

Baton funziona aggiungendo poche righe alle impostazioni del tuo agente: il server MCP che
l'agente avvia e, per Claude Code, un hook che parte quando l'agente finisce un turno. Non le
scrive mai senza mostrartele prima. Questa pagina spiega ogni riga.

## Quando la vedi

- Al primo avvio, al passo **I tuoi agenti**.
- In Impostazioni → Agenti, quando premi **Registra** o **Ripara** per un agente.

Per ogni agente che ha trovato, la schermata elenca le modifiche che farebbe. Ogni riga ha un
comando **Mostra** che apre il testo esatto: il file, il punto del file, e che cosa viene
scritto. Una riga già come deve essere dice **Già a posto**; una che verrebbe scritta dice
**Da modificare**. Non viene scritto nulla finché non premi **Accetta e registra**. **Non
ora** non scrive niente, e puoi registrare più tardi da Impostazioni → Agenti.

## Le modifiche per Claude Code

Sono tre, mostrate su due righe perché i due hook eseguono lo stesso comando.

### 1. La voce del server MCP, in `%USERPROFILE%\.claude.json`

> La voce del server MCP “handoff” in `C:\Users\tu\.claude.json`, che esegue
> `C:\Users\tu\AppData\Local\Baton\handoff-mcp.exe` con un timeout degli strumenti di 30
> minuti

Dentro `"mcpServers"`, Baton aggiunge:

```json
"handoff": {
  "type": "stdio",
  "command": "C:\\Users\\you\\AppData\\Local\\Baton\\handoff-mcp.exe",
  "args": [],
  "env": {
    "HANDOFF_AGENT": "claude-code",
    "HANDOFF_TOOL_TIMEOUT_MS": "1800000"
  },
  "timeout": 1800000
}
```

- `command` è il server MCP che arriva con Baton. Claude Code lo avvia per ogni sessione; è
  il modo in cui l'agente apre un handoff e riceve le tue risposte.
- `HANDOFF_AGENT` dice al server in quale agente sta girando, così sa che cosa quell'agente
  può fare (per esempio se sa leggere un'immagine).
- `timeout` e `HANDOFF_TOOL_TIMEOUT_MS` sono il timeout degli strumenti, spiegato sotto.

### 2. Due hook (Stop e SubagentStop), in `%USERPROFILE%\.claude\settings.json`

> Due hook (Stop e SubagentStop) in `C:\Users\tu\.claude\settings.json`, stesso comando

Dentro `"hooks"`, Baton aggiunge una voce all'elenco `"Stop"` e la stessa voce all'elenco
`"SubagentStop"`:

```json
{
  "matcher": "",
  "hooks": [
    {
      "type": "command",
      "command": "\"C:\\Users\\you\\AppData\\Local\\Baton\\handoff-mcp.exe\" hook stop",
      "timeout": 5
    }
  ]
}
```

Quando l'agente sta per finire il turno, l'hook chiede a Baton — in circa due secondi al
massimo — se c'è qualcosa che aspetta questa sessione: un handoff che hai rimandato, una
richiesta che hai scritto, un esito che l'agente non ha ancora raccolto. Se sì, lo ricorda
all'agente una volta. Se Baton non risponde, o qualcosa non è chiaro, l'hook non dice niente e
l'agente si ferma come sempre. Gli hook che avevi restano dove sono; quello di Baton si
aggiunge accanto.

## La modifica per Codex

È una sola, perché Codex non esegue nessun hook alla fine di un turno; [Codex CLI](agents/codex.md)
spiega che cosa cambia.

### La voce del server MCP, in `%USERPROFILE%\.codex\config.toml`

> La voce del server MCP “handoff” in `config.toml`, che esegue
> `C:\Users\tu\AppData\Local\Baton\handoff-mcp.exe` con un timeout degli strumenti di 30
> minuti; i suoi strumenti sono approvati in anticipo, così Codex non chiede il permesso a ogni
> chiamata

Baton aggiunge questa sezione al file, o crea il file con questa sezione:

```toml
[mcp_servers.handoff]
command = 'C:\Users\you\AppData\Local\Baton\handoff-mcp.exe'
args = []
env = { HANDOFF_AGENT = "codex", HANDOFF_TOOL_TIMEOUT_MS = "1800000" }
default_tools_approval_mode = "approve"
tool_timeout_sec = 1800
```

- `command`, `HANDOFF_AGENT` e `HANDOFF_TOOL_TIMEOUT_MS` fanno quello che fanno per Claude
  Code, sopra.
- `default_tools_approval_mode = "approve"` permette a Codex di usare gli strumenti **di Baton**
  senza chiederti il permesso prima di ogni chiamata. Senza questa riga Codex si ferma a
  chiedere ogni volta, e una sessione avviata con `codex exec` li rifiuta del tutto. Vale solo
  per questo server: ogni altro server e ogni comando mantengono le regole di approvazione che
  hai impostato tu.
- `tool_timeout_sec` sono gli stessi 30 minuti del `timeout` di Claude Code, in secondi, che è
  l'unità in cui conta Codex.

## Il timeout

Un handoff può richiederti minuti, e l'agente lo aspetta. Claude Code dà a ogni chiamata di
uno strumento un limite di tempo, quindi Baton lo alza **solo per il suo server**, a 30
minuti, con il campo `timeout` di quella voce. Se un handoff dura di più non si perde nulla:
all'agente viene detto che l'handoff è ancora in corso, e lo riprende.

La variabile globale `MCP_TOOL_TIMEOUT`, che cambierebbe il limite di **tutti** i server MCP
di Claude Code, non viene mai scritta. Se l'hai impostata tu, Baton la lascia esattamente
com'è, all'installazione e alla rimozione.

## Che cos'altro succede quando accetti

- Baton crea `%USERPROFILE%\.handoff\channel.token` se non esiste: il segreto con cui il
  server e Baton si riconoscono. Solo il tuo utente di Windows può leggerlo.
- Prima di modificare un file, Baton ne salva una copia accanto, chiamata
  `<file>.handoff-backup-<data e ora>`.
- Tutto il resto del file rimane: ogni chiave e ogni valore che avevi, nello stesso ordine e
  con la stessa indentazione. Le righe vuote tra una voce e l'altra non vengono conservate. Nel
  `config.toml` di Codex restano anche i commenti e le righe vuote, e il modo in cui è scritto
  ogni valore.
- Claude Code e Codex leggono le loro impostazioni quando una sessione parte, quindi riavvia le
  sessioni che erano già aperte.

## Dove: questo utente o un progetto

Di norma Baton si registra per il tuo utente di Windows, così lo vede ogni sessione di Claude
Code. In Impostazioni → Agenti → **Dove** puoi scegliere invece **Un progetto** e indicare una
cartella: Baton scrive allora `.mcp.json` e `.claude\settings.json` dentro quella cartella,
con lo stesso contenuto, e lo vedono solo le sessioni che lavorano lì. Per Codex scrive lì
`.codex\config.toml`, che Codex legge solo per un progetto che hai segnato come attendibile in
Codex; Baton non ne segna mai uno al posto tuo.

## Tornare indietro

In Impostazioni → Agenti, **Rimuovi** cancella esattamente le righe di Baton: la voce del
server `"handoff"` e le due voci degli hook, riconosciute dal percorso del server di Baton nel
loro comando. Nient'altro viene toccato — gli altri server e hook restano, e `MCP_TOOL_TIMEOUT`
non è mai di Baton da ripristinare. Se la voce di Baton era l'unica dentro `"mcpServers"` o
`"hooks"`, anche la chiave rimasta vuota viene tolta. Per Codex, **Rimuovi** cancella la
sezione `[mcp_servers.handoff]`, riconosciuta allo stesso modo, e nient'altro del
`config.toml`. Le copie di backup restano dove sono.

## Lo stato di ogni agente

Impostazioni → Agenti mostra uno di questi:

| Stato | Significato |
|---|---|
| **Registrato** | tutte le righe di Baton ci sono e puntano a questa installazione |
| **Registrato in parte** | ne manca qualcuna; **Ripara** mostra che cosa aggiungerebbe |
| **Registrato in un'altra posizione** | le righe puntano a un server da un'altra parte, per esempio dopo che Baton è stato spostato; **Ripara** le riscrive |
| **Non registrato** | l'agente è installato e Baton non è nelle sue impostazioni |
| **Non presente su questo computer** | l'agente non è stato trovato |

**Ripara** passa da questa stessa schermata: mostra la modifica e aspetta te.

## L'altra domanda del primo avvio

Il primo avvio chiede anche se Baton deve partire quando accedi (*Avvia Baton quando accedo*,
spuntata). Questo scrive una voce di avvio per il tuo utente, chiamata `Baton`, che fa partire
Baton nascosto nella barra. Puoi disattivarla in Impostazioni → Generale.
