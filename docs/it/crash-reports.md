# Rapporti di crash

Baton non manda a nessuno niente su di te o su come lo usi — niente telemetria, niente
statistiche d'uso, niente rapporti di crash automatici, nemmeno di quelli da attivare a
richiesta.

## Se Baton si chiude in modo anomalo

Scrive un file di testo su questo computer, in `%APPDATA%\Baton\crashes\`, con il nome del
momento del crash in UTC: `2026-09-10T08-14-05Z.txt`. Il file contiene:

- la versione di Baton, e la versione e l'architettura di Windows;
- l'errore e il punto del codice di Baton in cui è avvenuto, con lo stack delle chiamate;
- le ultime 50 righe del registro interno di Baton, **solo con identificatori**: id degli
  handoff, orari, dimensioni, conteggi e codici di stato. I testi dei passi, le note, i valori
  e i segreti non arrivano mai a quel registro, quindi non possono finire in un rapporto di
  crash.

## Al riavvio successivo

Baton te lo dice: *Baton si è chiuso in modo anomalo l'ultima volta. Il rapporto è su questo
computer e non è stato inviato da nessuna parte: apri la cartella se vuoi mandarlo a mano.*
**Apri la cartella** apre `%APPDATA%\Baton\crashes\`; **Non ora** chiude l'avviso.

Se vuoi segnalare il crash, prima leggi il file — è testo semplice — e poi allegalo tu al tuo
messaggio.

## Toglierli

I file sono tuoi: cancellali quando vuoi. Disinstallare Baton con *Delete the application
data* spuntata rimuove la cartella (vedi [Installare su Windows](install-windows.md)).
