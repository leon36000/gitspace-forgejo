# FORGEJO-COMMENT-WIRE-001 — noms réels des webhooks Forgejo v17

Date : 25 octobre 2026. Dépôt produit : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, arbre local modifié. Aucun nettoyage ni commit n’a été fait.

## Source primaire vérifiée

Le scratch `/tmp/forgejo-v17-base-cas-poc` est au commit exact Forgejo `v17.0.0`, `0d791421f85ca13a63b665053d81ea765fbbc571`; son arbre est fortement modifié par les POC CAS/LFS. Les quatre fichiers cités dans `source-hashes.txt` sont propres dans ce scratch et leurs blobs sont enregistrés pour éviter d’étendre cette affirmation au reste de l’arbre.

`modules/webhook/type.go::HookEventType.Event` mappe les événements de revue vers les noms d’en-tête `pull_request_approved`, `pull_request_rejected` et `pull_request_comment`. `services/webhook/shared/payloader.go::AddDefaultHeaders` met `EventType.Event()` dans `X-Forgejo-Event` et signe le corps payload, pas l’en-tête. Les valeurs plus longues `pull_request_review_approved`, `..._rejected` et `..._comment` sont des valeurs de `review.type`, pas des en-têtes. Le notifier des revues utilise cette séparation.

Le même `Event()` émet `issue_comment` pour les commentaires d’issue et ceux d’une conversation PR. `CreateIssueComment` choisit le type interne selon `issue.IsPull`; `IssueCommentPayload` porte `issue`, `comment`, `is_pull` et un `pull_request` facultatif. Le récepteur précédent omettait `issue_comment` et acceptait à tort `pull_request_comment` avec le corps d’une conversation. Il acceptait également les valeurs de `review.type` comme si elles étaient des en-têtes. La lecture de source corrige donc plusieurs hypothèses du rapport historique WEBHOOK-REVIEW-SUBTYPE-BINDING-001; le rapport reste archivé, avec son interprétation marquée comme supersédée dans le checkpoint.

## Changement produit

- La capacité expose les six en-têtes retenus : `issues`, `issue_comment`, `pull_request`, `pull_request_approved`, `pull_request_rejected` et `pull_request_comment`.
- `issue_comment` choisit la ressource par la présence d’un objet `pull_request`; le numéro de PR doit égaler celui d’`issue`. Le booléen `is_pull`, s’il est présent, doit être booléen et cohérent. Un objet `comment` est obligatoire.
- Les commentaires de revue utilisent seulement `pull_request_comment`, exigent le `review.type` exact et sont toujours rattachés à une PR.
- Les en-têtes génériques `issues` et `pull_request` refusent la forme corps d’un événement commentaire. Les valeurs `pull_request_review_*` sont refusées comme en-têtes. Ces vérifications réduisent une reclassification de l’en-tête non signé à partir d’un corps signé; elles ne rendent pas l’en-tête lui-même authentique.
- Le driver de livraison Forgejo différencie les identifiants de type de hook configuré (constantes de hook) des noms réellement livrés dans l’en-tête. Les fixtures de processus et les tests Python ont été corrigés en conséquence.

## Vérifications

Le premier test d’ingestion, exécuté avant le code, a échoué avec HTTP 422 `unsupported_event` pour un `issue_comment` valide; après ajout, il passe. Un test du processus démontre aussi le refus d’un remplacement de l’en-tête par `issues` ou `pull_request` quand la forme du corps signé ne correspond pas.

Les commandes précises et leurs codes de sortie sont dans `checks-status.txt`; les sorties complètes sont les fichiers `*.log`. Résultats : ingestion 12/12, réconciliation 23/23, découverte de capacités 3/3, réception processus des événements Forgejo 2/2, refus processus de sous-type 1/1, tests Python du driver d’émission 26/26, contrôle de l’état Cargo 7/7. `py_compile`, rustfmt et `git diff --check` réussissent. La lecture Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`; aucune licence n’a été attribuée.

Ce lot n’a pas déclenché de hook sur une instance Forgejo vivante. Les tests du processus récepteur utilisent des payloads HMAC synthétiques structurés depuis les types Forgejo v17; ils ne démontrent pas l’émission réseau réelle. Les tests ciblés n’équivalent pas à une exécution complète de `--all-targets`. Aucun LLM, AgentOS, RAG, mémoire, benchmark ou service externe n’a été utilisé. Aucun code Forgejo n’a été copié dans le produit.

Le manifeste local est `SHA256SUMS`; `source-hashes.txt` fixe la version et les blobs Forgejo, et les logs donnent les résultats reproductibles.

Après synchronisation des chemins touchés, le manifeste global contient 3 244 lignes : 3 243 entrées syntaxiquement bien formées, dont 3 226 empreintes concordantes et 17 divergences sur 15 chemins; aucun chemin listé n’est manquant. Le placeholder historique malformé est préservé. Les divergences restantes sont dans d’anciennes preuves Cargo-LICENSE-STATE-EXPLICIT-001, les POC LFS 051/052/054, l’ancien rapport WEBHOOK-REVIEW-SUBTYPE-BINDING-001 maintenant marqué supersédé, et un test webhook non modifié; aucune n’est présentée comme vérifiée dans ce lot.
