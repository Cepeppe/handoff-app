# Storico ed esportazione

Baton tiene traccia di ogni handoff su questo computer. È la terza delle tre cose che puoi
verificare da te su Baton (vedi [Verificalo tu](verify-trust.md)): non solo che cosa è stato
fatto, ma tutto quello che è uscito dal pannello.

## Dov'è

Un unico database SQLite, `%APPDATA%\Baton\handoff.sqlite`. Non viene mai inviato da nessuna
parte.

## Che cosa tiene

Per ogni handoff:

- l'handoff come l'ha scritto l'agente, con ogni valore trattato come segreto sostituito da
  una maschera come `[treated as secret: api_key]`;
- che cosa è stato detto all'agente alla fine;
- quando è stato aperto e chiuso, l'agente e la cartella del progetto della sessione;
- i giri, che cosa hai fatto su ogni passo, e la verifica che l'agente ha riferito.

Per ogni invio — una domanda, uno screenshot, un rinvio, un abbandono:

- il testo **esattamente come è uscito**;
- per uno screenshot inviato come immagine, le sue dimensioni, il suo hash SHA-256 e i
  rettangoli nascosti — **mai i pixel**.

Niente viene cancellato in automatico: le voci restano finché non le cancelli tu.

## Impostazioni → Storico

L'elenco mostra gli handoff finiti, con quando sono stati aperti e chiusi e quanti giri
hanno richiesto. Un handoff ancora in corso sta nel pannello, non qui.

**Apri** mostra una voce: la tua richiesta se ne hai fatta una, la spec *come è stata salvata*,
*che cosa è stato detto all'agente*, i giri, e *che cosa è uscito da questa macchina*.

## Cancellare

- **Cancella** toglie una voce con i suoi giri, le note e gli invii.
- **Cancella tutto** toglie ogni handoff finito, ogni giro, ogni nota e ogni invio. Le tue
  impostazioni restano, e gli handoff ancora in corso non vengono toccati. Non si torna
  indietro.

Tutti e due chiedono conferma sul posto: **Sì, cancella**.

## Esportare

**Esporta JSON** scrive tutto lo storico in un file che scegli tu: un unico documento JSON con
ogni tabella, così com'è nel database. I valori mascherati restano mascherati. Sono i tuoi
dati, in una forma che qualunque programma sa leggere.

## Leggere il database da te

Qualunque strumento per SQLite può aprire `handoff.sqlite`. Aprilo in sola lettura mentre
Baton è in esecuzione, e non modificarlo: Baton fa affidamento su quello che ci ha scritto.
