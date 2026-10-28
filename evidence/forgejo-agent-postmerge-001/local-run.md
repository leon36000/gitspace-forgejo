# FORGEJO-AGENT-POSTMERGE-001 — exécuter la source relue après fusion

Date : 25 octobre 2026. Worktree examiné : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; modifications locales préservées.

## Question et commande

Le test d’acceptation du parcours précédent avait été lancé avant le merge, puis Forgejo confirmait seulement que le source fusionné était identique. Ce lot ajoute une validation comportementale après merge : une fois `main` relu via l’API de Forgejo, le runner recompile ces octets dans Bubblewrap et exécute le worker en boîte noire.

```text
python3 scripts/run_forgejo_live_pr_merge_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root "$PWD" --state-dir /tmp/forgejo-agent-postmerge-001 --non-admin-merger --sandbox-egress-during-merge
```

Le dossier d’état est conservé; une reprise doit employer un nouveau chemin direct inexistant sous `/tmp`. Les snapshots sources, sortie du vérificateur post-merge, réponses JSON safe, tests, versions et manifeste sont conservés ici.

Environnement : Linux `aarch64`; Python 3.12.3; Rust/Cargo 1.95.0; Bubblewrap 0.9.0; Forgejo `17.0.0+gitea-1.22.0` (Go 1.26.9), binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Résultat observé

Le runner crée et vérifie une PR fraîche, observe les refus de fusion sans approbation et par acteurs non autorisés, puis le gate Rust fusionne la PR #2 par HTTP 200. Le commit candidat est `012a293ab720b0cde552788c909bf962ee91e2eb`; le commit de merge et la tête `main` sont `91da7c45add2f16ad65476c9e65dcf6f1192229c`.

Après le merge, la réponse Forgejo pour `src/lib.rs?ref=main` est décodée. Le runner compile exactement ces octets avec le worker indépendant, dans un environnement Bubblewrap sans réseau, puis exécute les neuf cas d’acceptation depuis le processus de confiance. Résultat : `9 passed; 0 failed`. Le SHA-256 source relu/testé est `d2d3ae071c330a25aa59fba851ef59d51c3bf6da63138953e87c96b2b6274c45`; l’empreinte du worker est la même avant et après merge (`94347097f1c48eb26b881b494475284b2c9b924281f1b50571641a0ddaff4dd6`). Le readback confirme PR fusionnée, issue fermée et `main` au commit de fusion.

Le lot compose aussi la condition du lot précédent : une sonde Bubblewrap reste vivante avant et après l’appel du gate, avec les routes API, Web et `git-receive-pack` de la PR #2 bloquées dans le même namespace. L’API hôte relit la PR pendant cette attente (200). Le processus de sonde n’a que les FD 0/1/2, ne reçoit que le chemin d’un marqueur synthétique non monté, puis sort en code 0 avec stderr vide. La production du résultat `agent-egress-during-merge.json` est contrôlée par le runner, pas reconstruite d’après le message imprimé.

Le fixture du parcours observe la défaillance initiale de cinq critères (quatre passent), puis la correction passe les neuf cas avant le merge et à nouveau sur le source de `main`. Un statut succès mensonger et les chemins de fusion sans revue/avec un acteur non autorisé sont refusés.

## Vérifications et preuves

- Sous-suite Bubblewrap : 5/5 tests; suite Python complète : 170/170.
- `py_compile` et `git diff --check` : code de sortie 0.
- Le manifeste SHA-256 local passe 17/17 artefacts et snapshots.
- La validation finale relie le SHA post-merge au contenu `main`, aux neuf assertions exécutées et au worker compilé; aucun listener Forgejo ne subsiste sur `127.0.0.1:3130`.

## Limites

Le parcours est un seul cas logiciel (`clamp_inclusive`) piloté par le `local-test-adapter`; il ne prouve pas un AgentOS, le protocole MCP complet, un LLM, de la collaboration multi-agent, ni un gain comparatif. L’acceptance table est détenue par le runner de test local; la preuve ne couvre pas des critères gérés dans une instance de production. La sonde réseau est un harness déterministe, pas du code hostile; serveur et Bubblewrap partagent l’UID hôte. Les writers externes ne sont pas exclus et Forgejo v17 ne fournit pas ici de CAS atomique attendu-base. Les neuf cas ciblés ne remplacent pas des tests de propriétés ou une suite du projet complet. Aucun modèle, secret réel, service externe, changement upstream ou production n’a été utilisé.

Suite utile : examiner si le code produit expose déjà un test post-merge similaire au-delà de cette fixture, puis préparer un protocole apparié Forgejo+agent+CI conventionnels. Les métriques coût/tokens avec LLM restent bloquées tant qu’un budget numérique fiable n’est pas explicitement autorisé.
