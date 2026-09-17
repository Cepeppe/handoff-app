# Verificalo tu

Non dovresti doverti fidare della parola di Baton su niente, né leggerne il codice per
scoprirlo. Tre cose che lo riguardano puoi controllarle tu, sul tuo computer:

1. **Comunica solo in locale.** Il codice di Baton non fa nessuna connessione di rete. Lo
   verifichi qui sotto.
2. **Non esce niente senza anteprima.** Ogni screenshot ti viene mostrato, con i segreti
   nascosti, prima che tu decida che cosa inviare (vedi [Screenshot e privacy](screenshots-and-privacy.md)).
3. **Tutto quello che viene inviato resta registrato su questo computer.** Lo storico tiene il
   testo di ogni invio, e per un'immagine le dimensioni, l'hash e le zone nascoste (vedi
   [Storico ed esportazione](log-and-export.md)).

## Con che cosa parla Baton

- **Con il server MCP su questo computer**, `handoff-mcp.exe`, che il tuo agente avvia. I due
  parlano attraverso una named pipe (`\\.\pipe\handoff-…`) che solo il tuo account di Windows
  può aprire, e il server deve anche presentare il token in
  `%USERPROFILE%\.handoff\channel.token`, che solo il tuo account può leggere. Il token tiene
  fuori gli altri utenti di questo computer e le connessioni accidentali; non protegge da un
  programma malevolo già in esecuzione come te — quello è il confine di Windows. Il canale è
  descritto nella documentazione del server
  ([il canale interno](https://github.com/Cepeppe/handoff-mcp/blob/main/docs/channel.md)).
- **Con il tuo agente**, attraverso quel server, che gli parla sul suo input e output
  standard. Quello che invii all'agente, l'agente lo manda al suo fornitore del modello come
  parte della conversazione, come qualunque cosa tu gli scriva.

Il codice di Baton non parla con nient'altro. Contiene un solo punto che potrebbe aprire una
connessione di rete, tenuto per un futuro controllo degli aggiornamenti; in questa versione
nessuno lo chiama, e la versione non contiene nemmeno un client HTTP. Neanche la pagina dentro
la finestra di Baton può aprire connessioni: la sua content security policy è
`default-src 'self'`, senza `connect-src`.

## La finestra è disegnata da Microsoft Edge WebView2

Il pannello di Baton è una pagina web disegnata da **Microsoft Edge WebView2**, un componente
di Windows. Gira come programmi separati, `msedgewebview2.exe`, che Baton avvia e che si
fermano quando si ferma Baton. WebView2 fa connessioni di rete per conto suo, per i suoi scopi,
che il codice di Baton non chiede e non può vedere.

Baton avvia WebView2 con queste opzioni, che spengono la sua attività di rete in background:
`--disable-background-networking`, `--disable-component-update`,
`--disable-domain-reliability`, `--no-pings` (più le tre funzionalità che wry, la libreria con
cui Baton incorpora WebView2, spegne già: `msWebOOUI`, `msPdfOOUI`, `msSmartScreenProtection`).

### Che cosa rimane: misurato

Misurato il 10 settembre 2026 su Windows 11 (build 26200), WebView2 Runtime 152.0.4191.66, con
il comando qui sotto, osservando Baton e ogni processo che ha avviato per 70 secondi
dall'avvio:

| Processo | Senza le opzioni | Con le opzioni (questa versione) |
|---|---|---|
| `handoff-app.exe` (Baton) | nessuna connessione | nessuna connessione |
| `handoff-mcp.exe` (il server) | nessuna connessione | nessuna connessione |
| servizio di rete di WebView2 | una connessione HTTPS verso un indirizzo di Microsoft circa 60 secondi dopo l'avvio, e un socket UDP | niente |
| processo browser di WebView2 | una connessione HTTPS verso un indirizzo di Microsoft entro 5 secondi dall'avvio | **una connessione HTTPS verso un indirizzo di Microsoft entro 4 secondi dall'avvio** |

Quindi **rimane una connessione, ed è di WebView2**:

- **Chi la apre:** il processo browser di WebView2 — il `msedgewebview2.exe` figlio diretto di
  `handoff-app.exe`. Non il codice di Baton, e non la pagina: le richieste della pagina
  passerebbero dal servizio di rete di WebView2, che la policy qui sopra vieta e che non ne ha
  fatta nessuna. Sta fuori dalla parte di WebView2 che le opzioni controllano, ed è per questo
  che non la fermano.
- **Quando:** entro pochi secondi dall'avvio di Baton, con o senza un handoff aperto, con il
  pannello nascosto o visibile. È una sola connessione HTTPS (porta 443) che resta aperta.
- **Verso dove:** un indirizzo diverso da un avvio all'altro; sono stati misurati
  `40.101.113.8`, `40.101.113.65`, `52.97.135.50` e `52.97.232.194` (e, prima
  delle opzioni, `40.99.217.34`, `40.101.22.192`, `52.97.201.226`, `52.98.237.146` e
  `150.171.28.11`), tutti in blocchi di indirizzi che appartengono a Microsoft. Nessuno ha un
  nome DNS inverso e la cache DNS di Windows non conteneva nomi per loro, quindi questa pagina
  non può dirti di quale servizio di Microsoft si tratti. La connessione è cifrata; Baton non
  può vedere che cosa trasporta.

Quello che fa WebView2 dipende da Microsoft, non da Baton. Se vuoi vederlo da te, il comando
qui sotto lo mostra.

## Vedere tu ogni connessione

Incolla questo in una finestra di PowerShell — non servono diritti di amministratore — e
lascialo girare mentre usi Baton. Segue i due programmi di Baton e **ogni processo che
avviano**, WebView2 compreso, e stampa ogni connessione di rete di uno di loro la prima volta
che compare. `Ctrl+C` lo ferma.

```powershell
# Baton network watch: every connection of Baton and of each process it starts.
# Leave it running while you use Baton; Ctrl+C stops it.
$roots = @('handoff-app.exe', 'handoff-mcp.exe')
$seen = @{}
$last = ''
while ($true) {
  $all = @(Get-CimInstance Win32_Process)
  $tree = @{}
  foreach ($p in $all) { if ($roots -contains $p.Name) { $tree[[int]$p.ProcessId] = $p.Name } }
  do {
    $grew = $false
    foreach ($p in $all) {
      $id = [int]$p.ProcessId
      if ($id -ne 0 -and -not $tree.ContainsKey($id) -and $tree.ContainsKey([int]$p.ParentProcessId)) {
        $tree[$id] = $p.Name
        $grew = $true
      }
    }
  } while ($grew)
  $now = ($tree.Keys | Sort-Object | ForEach-Object { "$($tree[$_]) $_" }) -join ', '
  if ($now -ne $last) {
    $last = $now
    if ($now -eq '') { $now = 'no Baton process is running' }
    '{0:HH:mm:ss}  watching: {1}' -f (Get-Date), $now
  }
  Get-NetTCPConnection -ErrorAction SilentlyContinue |
    Where-Object { $tree.ContainsKey([int]$_.OwningProcess) -and $_.State -notin 'Listen', 'Bound' -and $_.RemoteAddress -notmatch '^(127\.|::1$|0\.0\.0\.0$|::$)' } |
    ForEach-Object {
      $key = "tcp $($_.OwningProcess) $($_.RemoteAddress) $($_.RemotePort)"
      if (-not $seen.ContainsKey($key)) {
        $seen[$key] = $true
        '{0:HH:mm:ss}  TCP  {1} (pid {2}) -> {3} port {4}, {5}' -f (Get-Date), $tree[[int]$_.OwningProcess], $_.OwningProcess, $_.RemoteAddress, $_.RemotePort, $_.State
      }
    }
  Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
    Where-Object { $tree.ContainsKey([int]$_.OwningProcess) -and $_.LocalAddress -notmatch '^(127\.|::1$)' } |
    ForEach-Object {
      $key = "udp $($_.OwningProcess) $($_.LocalPort)"
      if (-not $seen.ContainsKey($key)) {
        $seen[$key] = $true
        '{0:HH:mm:ss}  UDP  {1} (pid {2}) opened a socket on port {3}' -f (Get-Date), $tree[[int]$_.OwningProcess], $_.OwningProcess, $_.LocalPort
      }
    }
  Start-Sleep -Milliseconds 500
}
```

Che cosa aspettarsi in questa versione (il comando stampa in inglese):

- una riga `watching:` con `handoff-app.exe`, i processi `msedgewebview2.exe` che ha avviato e
  gli eventuali `handoff-mcp.exe` che i tuoi agenti stanno usando;
- **una** riga come `TCP  msedgewebview2.exe (pid 39120) -> 40.101.113.8 port 443, Established`:
  è la connessione di WebView2 descritta sopra;
- **nessuna riga con `handoff-app.exe` o `handoff-mcp.exe`**, comunque tu usi Baton. Una riga
  così sarebbe il codice di Baton che apre una connessione, cosa che questa versione non deve
  fare mai.

Il comando lascia fuori le connessioni verso questo computer stesso (`127.0.0.1` e `::1`):
WebView2 può provarne una per un attimo quando parte, e non esce mai dalla macchina. Una
connessione che si apre e si chiude in meno di mezzo secondo può sfuggire tra due controlli;
la prova con il firewall qui sotto non ha questo limite, per i programmi di Baton.

## La prova con il firewall

Questa prova mostra che i programmi di Baton non hanno bisogno della rete: bloccali, usa Baton
normalmente, e non si rompe niente.

1. Apri PowerShell **come amministratore**, con lo stesso utente di Windows che ha installato
   Baton, e blocca i due programmi:

   ```powershell
   New-NetFirewallRule -DisplayName 'Block Baton' -Direction Outbound -Action Block -Program "$env:LOCALAPPDATA\Baton\handoff-app.exe"
   New-NetFirewallRule -DisplayName 'Block Baton server' -Direction Outbound -Action Block -Program "$env:LOCALAPPDATA\Baton\handoff-mcp.exe"
   ```

   (Le stesse regole si possono creare a mano in *Windows Defender Firewall con sicurezza
   avanzata* → *Regole in uscita* → *Nuova regola* → *Programma*.)
2. Usa Baton per un handoff completo, con uno screenshot inviato sia come immagine sia come
   testo.
3. **Risultato atteso:** non si rompe niente, e Impostazioni → Rete in Baton dice ancora
   *Nessuna connessione registrata.* Il comando di controllo qui sopra non stampa nessuna riga
   per `handoff-app.exe` o `handoff-mcp.exe`.
4. Togli le regole:

   ```powershell
   Remove-NetFirewallRule -DisplayName 'Block Baton', 'Block Baton server'
   ```

Le regole non coprono WebView2: è un programma diverso, `msedgewebview2.exe`, condiviso con
altre applicazioni del tuo computer, e bloccarlo le romperebbe. Per questo il comando di
controllo segue invece l'intero albero dei processi.

In questa versione il numero di connessioni atteso dai programmi di Baton è **zero**. Una
versione futura con il controllo degli aggiornamenti ne farà esattamente una, verso un solo
dominio fisso, e questa pagina lo nominerà allora.

## La pagina Rete

Impostazioni → Rete elenca ogni connessione che fa il codice di Baton, con data, dominio e byte
inviati — ognuna viene registrata prima che venga inviato qualunque cosa. In questa versione
l'elenco è sempre vuoto, perché nessuno chiama l'unico punto che potrebbe connettersi. La
pagina non può elencare le connessioni di WebView2: non le fa il codice di Baton. La pagina è
quello che Baton dice di sé; il comando di controllo e la prova con il firewall sono il modo in
cui lo verifichi.

## Che cosa controllano gli sviluppatori a ogni modifica

- La versione distribuita non contiene un client HTTP; un test fallisce se una parte del codice
  di Baton diversa dal suo unico modulo di rete nomina un tipo di rete, e un altro se qualcosa
  chiama quel modulo.
- Un test fallisce se la content security policy della finestra acquista un `connect-src`, o se
  una finestra avvia WebView2 senza le opzioni qui sopra.
- La suite end-to-end, che fa passare un agente vero attraverso ogni tipo di handoff, fallisce
  se nella pagina Rete viene registrata una qualunque connessione.
