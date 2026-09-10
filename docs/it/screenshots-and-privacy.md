# Screenshot e privacy

Uno screenshot è l'unica cosa che Baton invia senza che tu l'abbia scritta, quindi è lì che
Baton sta più attento. Non viene mai catturato niente se non premi **Screenshot**, e non esce
niente finché non hai visto esattamente che cosa uscirà.

## La cattura

**Screenshot** offre due scelte, ogni volta:

- **Schermo intero** — il monitor sotto il puntatore del mouse.
- **Seleziona un'area** — trascina un rettangolo, anche a cavallo di più monitor; `Esc`
  annulla.

L'ultima scelta che hai usato è evidenziata, ma Baton non sceglie mai al posto tuo. Il
pannello si nasconde mentre lo schermo viene catturato, così non finisce nella foto. Tra una
cattura e l'altra niente guarda il tuo schermo.

## L'anteprima

L'immagine compare subito, mentre Baton la legge: *Sto leggendo la cattura…*. I due pulsanti
di invio restano disattivati finché non ha finito.

### Leggere il testo, su questo computer

Per trovare i segreti Baton legge il testo nell'immagine — in locale, e senza rete:

- con **il riconoscimento del testo di Windows** quando Windows lo ha per la lingua (la
  funzionalità di *riconoscimento ottico dei caratteri* arriva con un language pack; di solito
  c'è quella inglese);
- altrimenti con **ocrs**, un motore di riconoscimento del testo incluso in Baton, che legge
  meglio l'inglese.

L'anteprima dice quale dei due l'ha letta (*Letta da windows*, *Letta da ocrs*). Nessun testo e
nessuna immagine vengono inviati da qualche parte per essere letti.

### Che cosa viene nascosto

| Riquadro | Perché | Puoi toglierlo? |
|---|---|---|
| pieno, rosso | un segreto **certo**: testo che corrisponde a un formato pubblico di chiavi, token, chiavi private, segreti di webhook e token firmati | no |
| tratteggiato, ambra, *Potrebbe contenere un segreto* | un segreto **sospetto**: una stringa lunga dall'aspetto casuale, una lunga stringa esadecimale o in base64, oppure un valore accanto a una parola come *key*, *secret*, *token*, *password* | sì: **Mostra di nuovo**, e **Nascondi di nuovo** |

Un riquadro copre l'intera riga di testo in cui è stato trovato il segreto. Un valore che ti ha
dato l'handoff stesso, e che non è un segreto, non viene segnalato.

Puoi anche **Nascondi una zona** trascinandoci sopra, e **Ritaglia** l'immagine (**Annulla il
ritaglio** la rimette com'era).

## Inviare l'immagine o il testo

I due pulsanti stanno uno accanto all'altro; nessuno dei due è quello predefinito.

- **Invia immagine** manda l'immagine. Viene ridotta in modo che il lato più lungo sia al
  massimo 1600 pixel, e i riquadri nascosti vengono dipinti di nero pieno *dopo* la riduzione,
  così sotto non resta traccia del testo. Il pulsante non c'è quando l'agente non sa leggere le
  immagini.
- **Invia testo** manda il testo che Baton ha letto, in un riquadro che puoi modificare. Sotto
  il riquadro lo vedi *come verrà inviato*: i segreti certi sostituiti da `[REDACTED:<kind>]`,
  quelli sospetti da `[REDACTED:suspected]`, a meno che tu non li abbia mostrati di nuovo.

Per uno schermo grande Baton suggerisce il testo: *Questo schermo è grande. Inviarlo come
testo spesso è più chiaro per l'agente e costa meno alla sessione.* Un'immagine grande costa
di più alla sessione dell'agente, e alcuni agenti tagliano un'immagine troppo grande.

Il commento facoltativo (*Qualcosa da dire al riguardo*) viene controllato come tutto quello che
scrivi: un segreto certo viene tolto, uno sospetto viene evidenziato e inviato come l'hai
scritto.

Se non è stato possibile leggere la cattura, l'anteprima lo dice — *non è stato nascosto niente
in automatico* — e lascia la decisione a te: nascondi a mano quello che serve, poi invia
l'immagine, oppure scartala. **Invia testo** non viene offerto, perché non c'è testo.

## Dove va

Quello che invii va al tuo agente, attraverso il server MCP in esecuzione su questo computer.
Il tuo agente lo manda poi al suo fornitore del modello come parte della conversazione,
esattamente come qualunque cosa tu scriva all'agente. Baton non lo manda da nessun'altra parte.

La cattura resta in memoria solo finché non la invii o la scarti. Baton non scrive mai
l'immagine sul disco.

## Che cosa tiene lo storico

Per ogni invio, lo [storico locale](log-and-export.md) tiene:

- per il testo, il testo **esattamente come è stato inviato**;
- per un'immagine, le sue dimensioni, il suo hash SHA-256 e i rettangoli nascosti — **mai i
  pixel**.

## I segreti in un handoff

Indipendentemente dagli screenshot, il server MCP controlla ogni handoff, prima che Baton lo
mostri, cercando testo che corrisponda agli stessi formati certi. Un valore così appare
mascherato (`••••••`): **Copia** copia il valore vero, **Mostra** lo rivela per dieci secondi,
e lo storico e i runbook tengono solo una maschera come `[treated as secret: api_key]`.

Le tue **note** sono tue: vengono conservate come le hai scritte, e un segreto certo al loro
interno viene sostituito quando le note vengono riferite all'agente alla fine.
