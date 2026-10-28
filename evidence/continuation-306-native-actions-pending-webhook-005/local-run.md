# CONTINUATION-306 — événement natif Forgejo Actions `workflow_run_waiting`

Date : 27 octobre 2026. Worktree local à HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié. Aucun code Rust de production, Forgejo ou GitSpace n’a été copié ni modifié.

## Question et résultat

Forgejo v17 peut-il fournir un événement `workflow_run` réellement émis, avec un état non terminal, au receiver et au reconciler Rust sans runner ni intégration LLM ? Oui, pour un `workflow_dispatch` en attente : une instance jetable Forgejo `17.0.0+gitea-1.22.0` a enregistré une workflow `.forgejo/workflows/pending.yml`, puis l’API a créé le run `id=1`, `index_in_repo=1`, HTTP 201. La requête HTTP de dispatch ne falsifie pas un webhook; Forgejo lui-même a exécuté son chemin de notification et envoyé les événements.

Le receiver réel a accepté avec HMAC et attestation le webhook `workflow_run_waiting` (`new_run_attempt`); sa tâche Forgejo liée au delivery ID a été acquittée HTTP 202. Le readback Forgejo contient deux tâches pour ce hook, mais le reçu ne conserve ni le type ni le lien événement→tâche de la seconde; aucun résultat `workflow_job_waiting` n’est revendiqué. Le CLI `forgejo-reconcile-local`, construit depuis le checkout Rust courant, a réconcilié la livraison du run par exactement un GET API en lecture seule avec un compte `forge-reader` non administrateur sans permission push. Verdict : `current_state_read`, état courant `waiting`, SHA courant identique au run. La sortie indique `independent_verification=false` et `merge_authorizing=false`.

La suite d’intégration n’a lancé ni enregistré de runner. Elle ne prouve pas l’exécution d’un job, une réussite CI, une vérification indépendante, une autorité de fusion ou une intégration AgentOS/LLM. Un run `waiting` est une observation d’état, jamais une preuve de succès.

## Source Forgejo vérifiée

Le dépôt scratch `/tmp/forgejo-v17-base-cas-poc` est au commit officiel v17.0.0 `0d791421f85ca13a63b665053d81ea765fbbc571`; l’arbre scratch contient d’autres POC locaux. Les sources utilisées pour expliquer le comportement ont été vérifiées à ce commit et sont sans diff local pour les fichiers cités :

- `routers/api/v1/repo/action.go::DispatchWorkflow` lit le workflow du ref, appelle `Workflow.Dispatch`, puis retourne `id`, `run_number` et les jobs quand `return_run_info` est demandé (lignes 824–871).
- `services/actions/workflows.go::Workflow.Dispatch` construit un run avec `StatusWaiting`, insère le run et retourne son identité (lignes 147–163, 205–209).
- `services/actions/run.go::propagateNextRunAttempt` recharge le run créé puis appelle `notify_service.NewWorkflowRunAttempt` (lignes 77–90).
- `services/webhook/notifier.go::NewWorkflowRunAttempt` prépare le payload `{action: new_run_attempt, run}` puis sélectionne l’en-tête à partir du statut (lignes 894–924; correspondance des statuts aux lignes 1147–1162).

Cette lecture ciblée du chemin de dispatch/notification ne constitue pas un audit exhaustif du moteur Actions ou des runners. Le binaire Forgejo local annonce v17.0.0 et son SHA-256 est `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build demeure inconnu. Aucun test Go du scratch n’a été lancé.

## Commande et environnement

Commande du parcours final :

```text
python3 scripts/run_forgejo_native_actions_pending_webhook_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-actions-pending-20261027-005 --evidence-dir evidence/continuation-306-native-actions-pending-webhook-005
```

Le dossier d’instance était neuf et privé sous `/tmp`; le test s’est arrêté si le port fixe `127.0.0.1:3130` était occupé. La configuration locale met `[actions] ENABLED = true` et limite l’allowlist de webhooks à `127.0.0.1`. Elle crée un dépôt privé portant le nom fixe attendu par le profil de readback, un compte vérificateur distinct en lecture seule et une workflow dont le label `runs-on` n’a aucun runner enregistré dans cette instance. Le script n’inscrit pas de runner. Le code du réconciliateur est compilé hors ligne/verrouillé dans Bubblewrap 0.9.0 avec `--unshare-net`, source/registre/sysroot montés en lecture seule, puis appelé comme processus local.

Rust/Cargo : `1.95.0`; le reçu du build contient les versions et empreintes des deux exécutables. L’instance utilisée est SQLite mono-hôte. Aucune donnée privée réelle ou requête modèle n’a été utilisée.

## Vérifications

- Test Python ciblé : `ForgejoActionsConfigurationTests` et `NativeActionsDeliveryBindingTests`, **3 réussis, 0 échec**. Ils vérifient que Actions reste désactivé par défaut, que l’activation est explicite et que l’identité/statut/repository d’un événement natif incohérent est refusé.
- `py_compile` des deux scripts et du module de tests : réussi.
- Parcours Forgejo live local : **1 run pending**, **2 webhooks natifs reçus et acquittés**, réconciliation du run par **1 GET**, état final courant `waiting`, SHA identique.
- Désactivation du hook confirmée; Forgejo et receiver arrêtés avec code 0; les deux listeners sont absents au readback.
- `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` vérifiés séparément pour ce lot; le Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed` et `NOASSERTION`.
- Sorties fraîches des tests, de `py_compile`, du formatteur, du diff-check, de l’état de licence et des versions : `unit-tests.log`, `pycompile.log`, `format-final.log`, `diff-check.log`, `license-state.log`, `toolchain.txt`, `commands.txt` et le reçu structuré `checks.json`. Les empreintes du code produit sont dans `source-sha256.txt`; les blobs amont sont dans `forgejo-source-sha256.txt`.
Les contrôles finaux après correction de la portée documentaire passent aussi : `rustfmt-final.log`, `diff-check-final.log`, `license-state-final.log`, `checks.json` et le contrôle `sha256sum -c source-sha256.txt`. Le test Python et `py_compile` ne sont pas rejoués après ces seules corrections documentaires.

## Échec utile et correction du harnais

Un premier run sur un dépôt de fixture au nom différent a été refusé par l’adaptateur comme `resource_out_of_scope`, avec zéro lecture upstream. Ce refus était le comportement attendu du profil fixe; les détails ne provenaient pas d’un bug Forgejo. Le harnais a été corrigé pour créer le nom exact `forge-admin/agent-forge-core`, sans élargir le profil ni sa portée. Un essai intermédiaire suivant a confirmé que le CLI renvoyait `current_state_read` mais a fait échouer une assertion du harnais : le JSON expose `resource.run_id` à plat, pas `resource.id.run_id`. Après correction de cette assertion de test, le parcours final a réussi. Les projections JSON expurgées du refus scope et de l’assertion du harnais sont jointes; elles omettent entièrement les `stage_facts` et les valeurs privées.

## Limites et suite

`n=1`, une instance, un run; état en attente uniquement. Le reçu compte une seconde tâche Forgejo sans en enregistrer le type ni la liaison au payload; aucune assertion sur `workflow_job_waiting` ni son passage au reconciler n’est établie. Aucun runner réel, étape exécutée, statut terminal, CI conventionnelle, LLM/AgentOS, PR, merge, baseline appariée, mesure de tokens/coût ou valeur comparative n’est démontré. L’instance et le binaire locaux ne prouvent pas les versions/configurations futures ni la sécurité d’une forge de production. La compilation Bubblewrap de confiance isole le build; elle n’est pas un test de sandbox d’un agent hostile.

Suite indépendante : si un runner ou un AgentOS déjà configuré et autorisé devient localement disponible sans accès/coût nouveau, tester son exécution sur une tâche fixture avec une vérification indépendante. En attendant, poursuivre les refus du contrat et la documentation sans fabriquer de statut CI. La mission reste incomplète; licence GitSpace indéterminée, aucune licence produit choisie, comparaison générale non démontrée.
