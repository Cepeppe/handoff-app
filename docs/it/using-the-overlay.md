# Usare il pannello

## Il pannello

Baton è un pannello stretto che resta sopra le altre finestre. Trascinalo dalla sua barra del
titolo; Baton ricorda dove l'hai messo su ogni monitor. Chiuderlo lo nasconde nella
barra — per uscire da Baton usa **Esci** nel menu della sua icona.

A destra nella barra del titolo ci sono tre pulsanti, e sono gli stessi tre in ogni schermata
di Baton:

| Pulsante | Che cosa fa |
|---|---|
| **Riduci a icona** | mette via Baton. Non si perde niente: l'icona nella barra lo riapre |
| **Rimpicciolisci a barra** | riduce il pannello alla barra di una riga, senza aspettare che tu faccia clic altrove. C'è mentre stai seguendo un passo |
| **Espandi** | allarga la finestra, con i tuoi handoff elencati a sinistra. **Ripristina** la riporta com'era |

Un pulsante che nella schermata in cui sei non farebbe niente resta al suo posto, in grigio, e
il suggerimento dice perché. Anche un doppio clic su una parte vuota della barra del titolo
espande e ripristina.

La vista espansa dura finché Baton è in esecuzione: non viene ricordata tra un avvio e
l'altro, e torna al pannello stretto da sola mentre sei nelle Impostazioni o stai scrivendo
una nuova richiesta. Un clic altrove la riduce comunque alla barra, e un clic sulla barra
riapre la forma in cui eri.

L'icona nella barra c'è sempre. Il suo menu ha **Mostra**, **Nuova richiesta**,
**Impostazioni** ed **Esci**. Un pallino sull'icona vuol dire che ci sono handoff aperti, e il
suggerimento dice quanti.

Quando un agente apre il primo handoff, il pannello viene in primo piano da solo. I successivi
aggiungono una scheda e un contatore (*2 nuovi*) senza prendere il focus.

## Le schede

Ogni handoff è una scheda, con l'agente e la cartella del progetto. Gli handoff che aspettano
te — quelli in sospeso e gli esiti che nessuno ha raccolto — stanno in un gruppo **In attesa**
che puoi chiudere.

## Un passo

Un handoff è un elenco di passi, mostrati uno alla volta: *Passo 2 di 4*, oppure
*Correzione · 1 di 2* quando l'agente ha mandato passi corretti dopo una verifica fallita.

- **Il testo** del passo, come l'ha scritto l'agente, con un avviso sopra quando l'agente ne
  ha dato uno.
- **I valori** che ti serviranno, ognuno con **Copia** (o **Copia questo** per un elemento di
  un elenco).
- **Apri** per la pagina di cui parla il passo. Si apre nel tuo browser.
- **I segreti** che creerai in questo passo sono elencati sotto *Incollali tu, noi non li
  vediamo mai*, con **Apri il file** per il file in cui vanno. Baton non li vede mai: li
  incolli tu.
- **I valori mascherati**: un valore passato dall'agente che sembra un segreto appare come
  `••••••`. **Copia** copia il valore vero; **Mostra** lo rivela per dieci secondi.

## I pulsanti

| Pulsante | Che cosa fa |
|---|---|
| **Fatto** | il passo è fatto; dopo l'ultimo il giro è finito e l'agente viene avvisato |
| **Chiedi** | fa una domanda all'agente su questo passo; la risposta compare sullo stesso passo |
| **Screenshot** | mostra all'agente quello che vedi, dopo un'anteprima obbligatoria (vedi [Screenshot e privacy](screenshots-and-privacy.md)) |
| **Nota** | una nota per te su questo passo; le note vengono riferite all'agente alla fine |

Altri tre stanno sotto **Altro**, l'ultimo pulsante della riga, perché chiudono qualcosa
invece di farla andare avanti:

| Pulsante | Che cosa fa |
|---|---|
| **Salta** | salta il passo; all'agente viene detto quali passi sono stati saltati |
| **Rimanda** | ci tornerai più tardi; l'agente viene avvisato, e riprende quando sei pronto |
| **Abbandona** | ferma l'handoff; puoi dire perché |

**Riprendi** e **Chiudilo** non stanno mai sotto **Altro**: su un handoff in sospeso e su un
esito che nessuno ha raccolto sono il pulsante principale.

Quello che scrivi in **Chiedi**, **Rimanda** e **Abbandona** viene mostrato sotto *Quello che
leggerà l'agente* prima che tu lo invii. Un segreto che Baton riconosce con certezza viene
tolto; le parole che *potrebbero* essere un segreto vengono evidenziate, e inviate come le hai
scritte — modificale se non devono partire.

Un handoff rimandato due volte è **in sospeso**: aspetta nel gruppo In attesa finché non
premi **Riprendi**.

## La barra ridotta

Quando fai clic fuori dal pannello, si riduce a una riga: a che passo sei, il suo testo, e
**Fatto**, **Chiedi** e **Screenshot** — gli ultimi due come icone, perché la barra è larga
una riga. Se premi **Screenshot** lì, si apre prima il pannello e la scelta della cattura
compare in quello.

All'estremità destra ci sono **Riduci a icona** e **Apri il pannello**. Anche un clic sulla
riga riapre il pannello, e lo riapre nella forma in cui l'avevi lasciato: stretto o espanso.

Se il pannello resta aperto quando non dovrebbe, Impostazioni → Generale → Pannello può
ridurlo anche qualche secondo dopo il tuo ultimo clic.

## Che cosa dicono gli avvisi

| Avviso | Significato |
|---|---|
| In attesa della spec dell'agente | hai aperto una richiesta e l'agente non ha ancora risposto con i passi |
| L'agente lo riprenderà al prossimo resume | in questo momento l'agente non sta aspettando questo handoff; quello che fai viene conservato e consegnato quando torna |
| Inviato all'agente, in attesa della risposta | la tua domanda o il tuo screenshot sono all'agente |
| Rimandato; l'agente tornerà | l'hai rimandato |
| In sospeso; riprendilo quando vuoi | l'hai rimandato due volte |
| Ora l'agente deve verificare: … | hai finito i passi e l'agente sta controllando il risultato |
| Sessione staccata; l'esito arriverà al prossimo resume | la sessione dell'agente è finita; quello che fai viene consegnato alla prossima sessione che riprende questo handoff |

## Come finisce un handoff

Quando premi **Fatto** sull'ultimo passo, l'agente viene avvisato. Se l'handoff dice come
controllare il risultato, l'agente lo controlla e la scheda mostra che cosa ha riferito, con
l'etichetta *dichiarato dall'agente*:

- **Verificato** — l'agente ha controllato e ha funzionato.
- **Fallito** — non ha funzionato. L'agente può mandare passi corretti: un nuovo giro,
  *Correzione 1 di 2*.
- **Non verificato** — non è arrivato nessun esito, o non in tempo o non prima che la
  sessione dell'agente finisse; la scheda dice quale dei due.
- **Confermato da te** — non c'era niente che l'agente dovesse controllare.
- **Abbandonato** — l'hai fermato tu.

Un esito finito che nessun agente ha raccolto per sette giorni viene mostrato come *Nessuno
ha raccolto questo esito.*, con **Chiudilo** e **Copia l'id**.

## Chiedere tu un handoff

Quando stai per fare qualcosa in cui l'agente dovrebbe guidarti, premi `Ctrl+Alt+H` da
qualunque punto (o **Nuova richiesta** nel menu della barra). Baton chiede *Cosa stai per
fare?* e per quale sessione. `Invio` manda, `Esc` annulla.

Baton mette allora negli appunti una riga per l'agente e prova a portare in primo piano il
terminale dell'agente, così puoi incollarla. Quando non ci riesce — alcuni terminali non glielo
permettono — una notifica dice *Richiesta copiata: incollala nella sessione …*. Se non la
incolli, all'agente viene ricordata alla fine del suo turno. Se non c'è nessuna sessione
aperta, la richiesta aspetta la prima che parte.

Quando l'agente risponde, l'handoff prende il posto della tua richiesta. Se è stato collegato
alla richiesta sbagliata, **Cambia** ti fa scegliere quella giusta.

Se un altro programma usa già `Ctrl+Alt+H`, Baton non la prende e ti chiede una volta un'altra
combinazione. Puoi cambiarla quando vuoi in Impostazioni → Generale → Scorciatoia.

## Di quale sessione si tratta?

Se Baton non riesce a capire da quale di due sessioni che lavorano nella stessa cartella è
arrivato un hook, una scheda chiede *Di quale sessione si tratta?*. Scegli quella in cui stai
lavorando, oppure *Non lo so*.

## Quando Baton non è in esecuzione

L'agente può comunque passarti del lavoro: il server MCP risponde con l'handoff sotto forma di
testo, e l'agente ti guida **nella chat**. È la modalità testo. Non compare nessun pannello,
non viene registrato niente, non esiste lo stato *verificato* e non c'è niente da riprendere
dopo — la chat è la traccia.

Avvia Baton e il prossimo handoff torna a usare il pannello. Una sessione già aperta trova
Baton entro circa 30 secondi; non serve riavviarla.

## La lingua

Baton segue la lingua di Windows se è inglese o italiano, altrimenti usa l'inglese.
Impostazioni → Generale → Lingua la cambia.
