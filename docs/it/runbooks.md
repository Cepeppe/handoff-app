# Runbook

Un runbook è una ricetta: i passi che hanno funzionato l'ultima volta che un certo lavoro
umano è stato fatto, così che la volta dopo l'agente possa partire da lì invece che da zero.

## Quando Baton ne scrive uno

Dopo ogni handoff che finisce **Verificato**, e dopo ogni handoff che finisce **Confermato da
te** (con un'etichetta di fiducia più bassa). Mai dopo uno fallito, non verificato o
abbandonato.

La ricetta è quello che hai fatto davvero: i passi che hai confermato, in ordine, compresi i
passi corretti di un giro successivo, senza quelli che hai saltato. Le tue note, e gli errori
che hanno portato a una correzione, restano come osservazioni sul passo a cui appartengono.

## Dove sono

In `%USERPROFILE%\.handoff\runbooks\`, un file JSON per ogni runbook, in un formato pubblico
documentato insieme al server MCP open source
([formato dei runbook](https://github.com/Cepeppe/handoff-mcp/blob/main/docs/runbook-format.md)).
Puoi aprirli, leggerli e copiarli. Restano anche se disinstalli Baton.

## Niente valori, solo i loro nomi

Un runbook non contiene mai i valori di un handoff — niente URL del tuo progetto, niente
chiavi. Dove un passo usava un valore, il runbook ne tiene il nome come segnaposto,
`{{endpoint_url}}`, con la frase del passo in cui compariva come descrizione. Un segreto
trovato in qualunque altro punto del testo viene sostituito da una maschera come
`[treated as secret: api_key]`.

## Come li usano gli agenti

Prima di scrivere un nuovo handoff, l'agente chiede al server MCP i runbook sullo stesso posto
e con lo stesso obiettivo. Il server controlla anche da solo quando arriva un nuovo handoff: se
un runbook corrisponde, lo offre all'agente come bozza da cui partire, e l'agente riempie i
valori dal tuo progetto. Un runbook corrisponde quando il posto è lo stesso e i due obiettivi
hanno in comune parole significative — una regola semplice, senza indovinare. L'agente vede
anche quando il runbook è stato verificato l'ultima volta, per giudicare quanto è recente.

## Quando l'handoff di un runbook fallisce

Un handoff partito da un runbook che fallisce segna il runbook come *Ultima esecuzione fallita*
con la data. Baton non cancella mai un runbook da solo.

Se poi l'agente corregge i passi e la correzione funziona, Baton chiede: *Aggiorno il runbook …
con la sequenza corretta?* — **Aggiornalo** sostituisce i passi, **Tienili entrambi** lascia
il runbook com'era.

## Impostazioni → Runbook

L'elenco mostra ogni runbook con le sue esecuzioni, i suoi passi, quando è stato verificato
l'ultima volta, quando un'esecuzione è fallita l'ultima volta, e la sua fiducia: **Verificato**
o **Confermato da te**.

- **Apri la cartella** apre `%USERPROFILE%\.handoff\runbooks\`.
- **Cancella** sposta un runbook nel Cestino, dopo averlo chiesto.

Non c'è esportazione, importazione o libreria condivisa: un runbook è un file, e copiare il
file basta.
