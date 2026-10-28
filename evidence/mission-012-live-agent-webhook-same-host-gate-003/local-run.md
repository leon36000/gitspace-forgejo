# CONTINUATION-121 — même enfant MCP vivant pendant un gate indépendant

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, arbre de travail très modifié et préservé. Aucune modification de production, publication, suppression, commit, dépense LLM ou donnée privée envoyée à un service externe.

## Parcours observé

Le run 003 utilise une nouvelle instance Forgejo locale v17 et reçoit la livraison native `issues/opened` `97965d16-06f3-4831-b984-aadf8be95a3e`. Le reçu source, l’inbox, la base `617c1f49b4f43ffebc3eb1ae613548c3a62ed9c5`, la mission/exigence et le host MCP construit pour cette admission sont reliés dans `composed-run.json`, `composed-stage-summaries.json` et le manifeste d’admission.

Le vrai enfant Rust `mission-012-mcp-host` est maintenu au message JSON-RPC `shutdown`. Le superviseur dépose un marqueur privé nonce-lié qui identifie le PID `2340945`, son start-time Linux `18578061`, le chemin `/tmp/mission012-same-host-gate-123/build/host-artifacts/mission-012-mcp-host` et le superviseur PID `2340941`/start-time `18578060`. Le composeur relit ces bindings par `/proc`, calcule les octets de l’exécutable ouvert via `/proc/<pid>/exe`, et les lie au binaire MCP fourni par la provenance d’admission : SHA-256 `4bac67232c886a7ffe6a5817224980200bc74e62a6792077077b2dd25b5e96c6`.

Pendant que ce même PID/start-time reste vivant, le gate refuse les liens symboliques et fichiers inattendus, compare la base et la candidate, protège le fichier d’acceptation et n’autorise que `src/lib.rs`. Il relance l’acceptation avec `cargo test --offline --locked --test acceptance` dans Bubblewrap sur une candidate en lecture seule, avec cible et `/tmp` neufs, réseau/PID/user namespaces et sans home ni credential Forgejo. Le gate a rendu `pass`, sortie Cargo `2 passed; 0 failed`, en 240 ms; hash de sortie `5ac6218912917f6483e8f6c62da2e353c657380115a206d4272293d666899fdf`. Le reçu exact `gate-receipt.json` a le SHA-256 `a096c304b005378191e48812efca92f2fa839534f6c15db3c6b63e9bb15d67cb`, également lié au résumé final; le superviseur ne libère le host qu’avec le hash de ce reçu.

Après cette vérification, le même host finalise et l’étape PR indépendante s’exécute. La base reproduit l’échec protégé attendu (1 passé/1 échoué); seule `src/lib.rs` est changée (`173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`). La candidate passe les vérifications indépendante et pré-fusion (2/2 chacune). Forgejo crée et fusionne la PR locale #2; le candidat est `7894817b4ec2da74d45ec56cb3713afb0afce66b`, le commit fusionné et `main` relu sont `854ba0f18223d14003b2c4a66833046cb091d574`. La validation après fusion sur ce commit exact passe 2/2 et le readback relie la PR et l’issue fermées.

Le rapport composé a aussi reçu du sandbox agent `credential_visible=false` et les trois sondes directes API, Web et `git-receive-pack` bloquées. Le vérificateur de gate lui-même ne reçoit pas de credential pour Cargo. Cela observe la configuration Bubblewrap locale; cela n’établit pas une barrière universelle contre le même UID, root, un noyau compromis ou une évasion.

## Vérifications et reproduction

La source du run est le snapshot de composition conservé dans `composition-source-snapshot.zip`: 107 fichiers, manifeste SHA-256 `57ee4624518c6574eb33a06337094b3b0385acc10de38ce1ba46bd52d7e55498`, archive SHA-256 `878bddb31749d02fb7a08723266d4dc9c2d1c55a053fad3ee8809130acc11940`. Forgejo annonce `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; référence source officielle v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le commit exact de construction du binaire Forgejo n’est pas établi. Python `3.12.3`, Rust/Cargo `1.95.0`, Bubblewrap `0.9.0`. Namespace réseau de l’hôte : `net:[4026531833]`; namespace privé du run : `net:[4026535804]`.

Commande exécutée depuis la racine du worktree (l’outer Bubblewrap donne au composeur son namespace réseau privé; le runner exige explicitement le namespace hôte attendu) :

```sh
/usr/bin/bwrap --unshare-net --ro-bind / / --dev-bind /dev /dev --proc /proc \
  --bind /tmp /tmp --bind "$PWD" "$PWD" --chdir "$PWD" -- \
  /usr/bin/python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-same-host-gate-123 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-same-host-gate-003" \
  --execution-id live-mcp-123 \
  --allocation-id mission012-same-host-gate-123 \
  --budget-cap-usd 0.20 \
  --expected-host-network-namespace 'net:[4026531833]' \
  --agent-adapter local-test-adapter --sandbox-test-adapter \
  --same-host-gate-barrier
```

Les 18 artefacts du run et les pièces de reproduction sont persistés dans ce dossier. Le manifeste local append-only couvre 24 fichiers; ses dernières empreintes passent le vérificateur qui valide l’unique entrée la plus récente par nom et exige une couverture exacte. Il conserve deux anciennes empreintes remplacées de `append_manifests.py`, donc un `sha256sum -c` brut sur toutes les lignes signale ces deux entrées historiques comme périmées; l’entrée la plus récente correspond au fichier courant. Les manifestes séparés des admissions 001, 002 et 003 passent chacun `sha256sum -c` (10/10). Les suites Python ciblées passent 38/38 : composeur/barrière, adaptateur MCP, routage d’arguments, superviseur et wrapper sandbox; les journaux sont dans `targeted-tests.log`, `pycompile.log` et `diff-check.log`.

Deux tentatives antérieures ne sont pas comptées comme succès et restent intactes sous `/tmp` : run 001 a démontré que la fermeture EOF seule ne transmet pas `shutdown`; le superviseur a refusé la barrière (exit 77). L’adaptateur stdio envoie maintenant une requête JSON-RPC `shutdown` explicite avant de fermer le pipe. Le run 002 a atteint la vérification mais une variable mal nommée (`agent`) arrêtait le composeur avant la PR; le composeur emploie maintenant le résultat parsé. Les tests ciblés couvrent ces changements. Seul le run 003 est compté pour le parcours complet.

## Limites

`n=1`, `local-test-adapter`, `model_invoked_during_run=false`; tokens nuls/non mesurés, aucun coût LLM. Le comportement d’un agent LLM, AgentOS, l’impact sur les interventions, le temps global et le coût ne sont pas mesurés. Le même enfant est vivant pendant le gate d’acceptation seulement : après libération il finalise avant le stage PR et le merge contrôlé. Le processus Python trusted vérifie PID/start-time/exécutable sur cet hôte, mais ne constitue pas un superviseur résistant au même UID. Forgejo est une instance jetable locale dont le commit de construction n’est pas identifié. Cette preuve ferme uniquement le manque précis « le child webhook-admis vit pendant un gate indépendant »; elle n’établit pas de gain comparatif ni l’achèvement du projet.
