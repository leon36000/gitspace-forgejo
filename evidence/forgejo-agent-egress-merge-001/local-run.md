# FORGEJO-AGENT-EGRESS-MERGE-001 — gate de fusion pendant le refus d’egress

Date : 25 octobre 2026. Worktree examiné : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, conservé sans reset ni commit.

## Question vérifiée

Le lot [FORGEJO-AGENT-EGRESS-LIVE-001](../forgejo-agent-egress-live-001/local-run.md) prouvait le refus de routes Forgejo réelles depuis le namespace réseau Bubblewrap, mais seulement une lecture autorisée depuis l’hôte. Ici, le runner reprend une PR fraîche et garde le processus de sonde isolé vivant de part et d’autre de l’appel réel à l’autorité de fusion Rust. Ce run compose de nouveau la préparation, les refus, la revue et la fusion uniquement pour ajouter cette condition simultanée.

## Commande et outils

Commande exécutée :

```text
python3 scripts/run_forgejo_live_pr_merge_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root "$PWD" --state-dir /tmp/forgejo-merge-egress-concurrent-001 --non-admin-merger --sandbox-egress-during-merge
```

Le runner refuse de réutiliser un dossier d’état existant. Le dossier `/tmp/forgejo-merge-egress-concurrent-001` est conservé avec le journal et la base de l’instance jetable; une reprise indépendante doit choisir un nouveau chemin direct sous `/tmp`. La preuve safe, les instantanés des sources exécutées, la commande, les versions et les empreintes sont dans ce répertoire `evidence/`.

Environnement observé : Linux `aarch64`, Python 3.12.3, Rust/Cargo 1.95.0, Bubblewrap 0.9.0; binaire Forgejo `17.0.0+gitea-1.22.0`, construit avec Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Résultat

Sur la PR Forgejo #2, l’API hôte relit la PR pendant que la sonde attend dans le namespace séparé : HTTP 200. Le processus Bubblewrap reste vivant avant et après l’appel de gate. Il tente, avant puis après le gate, les routes exactes de cette PR : API `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/2`, Web `GET /forge-admin/agent-forge-core/pulls/2` et Git `POST /forge-admin/agent-forge-core.git/git-receive-pack`. Les trois sondes sont classées bloquées à chaque passage; l’inode du namespace ne change pas. Le processus ne voit pas le fichier canari synthétique et ses seuls descripteurs ouverts rapportés sont `0, 1, 2`.

Pendant cette attente, le processus de confiance appelle le vrai binaire Rust `--merge-local-test`. Il reçoit son credential par le pipe déjà prévu pour le gate; celui-ci n’est ni transmis à Bubblewrap ni inscrit dans les arguments ou sorties. Forgejo accepte le merge en HTTP 200. La sonde répète les trois tentatives, reconnaît le même état de blocage, reçoit `release`, sort avec code 0 et stderr vide. Le readback Forgejo confirme PR fermée/fusionnée, issue fermée, `main` au commit de merge `d22770b49873b583fb957c3a705059ddca7890f9`; le SHA du contenu source relu correspond au SHA testé avant fusion.

Le même run du fixture a observé une première vérification rouge (5 assertions d’acceptation échouent, 4 passent), puis le candidat corrigé passe les 9 tests d’acceptation. La soumission d’un faux statut de l’exécutant est refusée; le gate refuse aussi une fusion sans approbation et un acteur non autorisé. Ces étapes préexistantes sont rejouées par le runner, mais la nouveauté de ce lot est le processus isolé qui demeure actif autour de la vraie invocation du gate et le readback qui suit.

## Vérifications conservées

- Suite ciblée de sandbox : 5/5 tests, dont la séquence `ready → reprobe → release`, l’identité du dépôt/PR et les descripteurs ouverts.
- Suite Python du dépôt : 170 tests, 0 échec.
- `py_compile` des scripts et tests concernés, `git diff --check` : code de sortie 0.
- Validation supplémentaire des artefacts : `result.json`, gate HTTP 200/verdict `merged`, namespace identique avant/après, trois routes toujours bloquées, processus vivant avant/après, sortie propre, et SHA du readback source égal au SHA testé.
- Le manifeste SHA-256 local de ce dossier vérifie 16/16 artefacts, y compris les snapshots source; le manifeste global `evidence/SHA256SUMS` conserve son historique append-only.
- L’instance Forgejo a été arrêtée par le runner; aucun listener n’est présent sur `127.0.0.1:3130` après le run.

## Portée et limites

C’est une sonde de capacité déterministe `local-test-adapter`, pas le processus complet du protocole MCP ni un LLM. Elle n’exécute pas du code candidat hostile; aucun secret réel ne lui est remis. Les routes visées sont testées directement, mais cela ne constitue pas une preuve universelle de l’absence de tout trafic réseau possible. Le serveur et le bac à sable partagent l’UID hôte : aucune isolation multi-tenant n’est démontrée. Le test n’exclut pas les writers externes/concurrents; la valeur `external_writers_excluded` reste fausse et Forgejo v17 ne fournit pas ici de CAS de base atomique. Le test d’acceptation est exécuté avant le merge; le readback après merge compare le contenu, sans réexécuter le binaire compilé depuis `main`. Aucun coût de modèle, benchmark conventionnel ou gain produit n’est mesuré. Aucun code Forgejo/GitSpace n’est copié, aucune production n’est modifiée, aucun service externe n’est appelé.

Prochaine action : ajouter un contrôle boîte noire post-merge qui compile et exécute le contenu relu depuis `main`, puis consigner explicitement les critères encore impossibles à comparer sans agent/modèle et budget de coût autorisé.
