# CONTINUATION-357 — projection du jalon courant

Date : 28 octobre 2026. Dépôt local sur `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucune réinitialisation, suppression, commit ou publication n’a été effectuée.

## Changement

La réconciliation d’issues et de pull requests projetait labels et assignees depuis l’API, mais omettait le milestone courant. Le nouveau champ `canonical_state.milestone` est issu exclusivement du GET courant : absence → `unknown`, `null` → `unassigned`, objet validé → `assigned`. L’objet garde seulement `id`, `title` et `state`; ID positif, titre non vide sans caractères de contrôle et limité à 1 024 octets, état `open` ou `closed`. Une forme mal formée est une erreur de snapshot et ne devient jamais une réussite canonique. Les événements webhook restent des notifications périmées et ne fournissent pas la valeur.

La projection est activée pour les ressources issue/PR et le parent issue/PR relu d’un commentaire. Elle est étiquetée `source=current_forgejo_resource_api`, `freshness=current_api_read`, `trust=untrusted_user_content`; elle ne sert ni de critère d’acceptation ni d’autorisation de mission/fusion. Les schémas single/batch passent de 17/21 à 18/22 et capabilities de v24 à v25. Le contrat de capacités documente ressources, états observables, bornes et refus.

## Source Forgejo examinée

Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, checkout de lecture `/tmp/forgejo-v17-base-cas-poc`. Les fichiers ciblés ont été vérifiés identiques au commit amont (`git diff --exit-code`), sans copier de code :

- `modules/structs/issue.go:59`: `Issue.Milestone *Milestone`.
- `modules/structs/pull.go:19`: le struct de pull request expose aussi `milestone`.
- `modules/structs/issue_milestone.go:11-35`: ID, titre et état de l’objet API.
- `services/convert/issue.go::ToAPI` charge le milestone et appelle `ToAPIMilestone`; cette conversion projette ID, titre et état.

Hashes SHA-256, commit et source consultée : [forgejo-source-audit.log](forgejo-source-audit.log). L’examen est borné à ces symboles/fichiers; le checkout scratch est sale sur d’autres POC. Aucun test Go/serveur Forgejo n’a été lancé pour ce lot.

## Vérifications

- `current_api_milestone_projection_distinguishes_unknown_unassigned_and_current`: absence, `null`, objet valide, et snapshot périmé du webhook.
- `malformed_current_api_milestone_refuses_canonical_snapshot`: 7 formes mal formées refusées par `InvalidCurrentStateField("milestone")`.
- `actual_reconcile_cli_process_projects_current_issue_milestone_from_api`: 1/1 sous le runner Bubblewrap `--unshare-net`; le vrai receiver signe/journalise l’événement synthétique, le vrai CLI lit une fixture HTTP loopback, effectue un GET sans Authorization, retourne le milestone courant, omet le texte périmé du webhook et laisse le journal byte-identical. Voir [process-e2e.log](process-e2e.log).
- `cargo test --offline --locked --all-targets -- --test-threads=1`: 737 réussis, 0 échec, 166 ignorés, en 38 groupes de cibles. Les tests ignorés par défaut ne sont pas couverts par ce total. Sortie : [all-targets-final.log](all-targets-final.log).
- Trois suites Python touchées : 155 + 20 + 30 = 205 tests réussis. Elles couvrent les contrats d’émission, de reviewers et de rejeu; elles ne sont pas des parcours Forgejo live.
- Garde de licence Cargo : vérification `metadata_policy_verified`, puis 12/12 tests; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, marqueur `NOASSERTION`.
- `cargo fmt --all -- --check` et `git diff --check` passent. Journaux des suites Python, de licence et de formatage dans ce dossier.

Les premiers essais de suite ont trouvé un initialiseur `ReconciledState` incomplet et une assertion conservant les anciens schémas 17/21; ces contrats ont été mis à jour et la suite finale complète passe. Les journaux initiaux restent dans [all-targets.log](all-targets.log), distincts du run final.

## Limites et migration PC4

Le processus de bout en bout utilise un receiver local et un faux endpoint loopback sous sandbox réseau; il ne démontre pas l’émission native d’un Forgejo vivant, un LLM/AgentOS, une PR, une vérification indépendante, une fusion ni un gain mesuré. L’architecture globale et les critères de complétion restent ouverts.

Le transfert demandé vers `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` reste bloqué : `git ls-remote` sans credential échoue sur la résolution du FQDN. Le ping Tailscale vers PC4 réussit, mais `accept-dns` est désactivé localement, TCP/443, 80 et 22 sont injoignables; une requête HTTPS `--resolve` vers l’IP Tailscale a expiré après 12 s. Aucun credential du courriel n’a été utilisé, aucune référence distante n’a été lue et aucune écriture Git n’a été faite. Le PAT trouvé n’a pas de portée cible confirmée; sa valeur n’est pas conservée dans ce dépôt. Un extrait du connecteur courriel a inclus cette valeur dans sa sortie; il est recommandé de révoquer/renouveler le PAT. Ne pas utiliser un mot de passe administrateur initial. Détails de connectivité expurgés : [gitlab-reachability.md](gitlab-reachability.md).

Prochaine action : rétablir depuis PC6 la résolution MagicDNS et l’accès Tailscale au port HTTPS du GitLab; ensuite lire les références en lecture seule, vérifier le dépôt et son contenu, et préparer une branche `codex/` sans toucher à `main` ni force-push. Si le PAT reste de portée inconnue, utiliser un nouveau credential limité au projet plutôt que le compte root.
