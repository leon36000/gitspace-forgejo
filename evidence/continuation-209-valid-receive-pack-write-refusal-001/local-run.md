# CONTINUATION-209 — refus d’un push Git valide assemblé contre le POST receive-pack

## Résultat

CONT-208 prouvait un 403 direct avec un pkt-line flush vide. Ce run va plus loin : il crée un dépôt Git jetable, un commit synthétique, puis un packfile avec `git pack-objects --stdout --revs`. Le client assemble un paquet receive-pack pour créer `refs/heads/agent/read-only-git-write-probe` depuis l’OID nul, demande `report-status`, termine le flux de commandes par `0000` et ajoute le packfile. Le paquet envoyé fait 468 octets, le pack 321 octets, SHA-256 `80ce339c66400b8e902cc00b8791afe9af328a8eb0b54439da9114e8929d95bd`; l’OID proposé est `df68c71d0fe68aa8a24f2044f183913acefd3ae7`. La reproduction hors-ligne depuis le dépôt synthétique conserve exactement ce hash et calcule le SHA-256 du paquet complet `2f05fbc27ad26d350118687ee821e437fd0c48431c086f1125418f578a3409c4` (voir `valid-push-payload-reproduction.json`).

L’instance Forgejo privée renvoie HTTP 403 au POST direct avec la commande et le pack, et son access log associe la méthode POST, la route exacte et le 403. Le POST antérieur au pkt-line flush vide reçoit lui aussi 403; l’annonce GET receive-pack reçoit 403; le vrai `git push` termine en 128 pendant cette annonce. Les readbacks admin indiquent que la branche cible est absente avant et après l’ensemble des tentatives. La sortie Git ne contient ni token ni Basic credential. Le token read-only sert aussi à lire `main` par Git HTTP. Le receiver Rust reçoit le webhook `pull_request/review_requested`, puis le CLI retrouve la PR ouverte et l’équipe courante `[{"id":2,"name":"reviewers"}]`; aucun merge n’est demandé. Le run retourne `verdict=passed`, les processus terminent code 0 et aucun listener ne subsiste.

Run live exact :

```sh
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-team-review-cont209-state-001 \
  --evidence-dir evidence/continuation-209-valid-receive-pack-write-refusal-001 \
  --probe-read-only-git-write
```

Forgejo `17.0.0+gitea-1.22.0`, Go 1.26.9; binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit de build inconnu. La connexion HTTP est épinglée à `127.0.0.1` sans redirection. Le workspace temporaire `/tmp/forgejo-team-review-cont209-state-001` est privé mode 0700 et préservé. Aucun contenu utilisateur, secret, modèle ou service externe n’est utilisé.

## Tests et évolution

Test-first : les assertions ajoutées ont d’abord échoué parce que le validateur n’exigeait pas les statuts des POST vides et valides (code 1). Après l’implémentation, le helper de framing passe ses cas de format/ref/OID, le validateur passe les refus de succès, de statut absent, de route étrangère, d’effet et de fuite. Le module Python ciblé passe 13/13; `py_compile`, la garde Cargo sans choix de licence, ses tests, `cargo metadata --locked --offline --format-version 1 --no-deps` et `git diff --check` passent. Le Cargo build live utilise Bubblewrap 0.9.0, Rust/Cargo 1.95.0, source en lecture seule, namespaces réseau/PID et `--offline --locked`.

## Portée et limites

Un seul binaire/version, compte/scopes, dépôt, configuration et run final (`n=1`). Le paquet est encodé par le harnais à partir d’un pack généré par Git; Forgejo répond 403 avant que la preuve puisse établir le parsing ou le dépaquetage côté serveur. Cela vérifie le contrôle d’autorisation au POST avec une requête de push formée et la non-création observable de la ref, pas la sûreté du moteur receive-pack sur une requête autorisée ni tous les autres écrivains/protocoles/roles. Pas de SSH, push avec credential write, routes Web/API exhaustives, concurrence multi-writer, autorité universelle de merge, AgentOS ou LLM. Aucun gain comparatif n’est mesuré; le projet reste incomplet.

La première commande de reproduction hors-ligne a échoué (wrapper ayant suréchappé le newline d’entrée de `git pack-objects`, exit 128); elle reste consignée dans `pack-reproduction-attempt-001.md` avec son log vide. La reprise corrigée `pack-reproduction-retry-002.log` reproduit exactement le hash du pack, son OID, sa taille et le hash du body. Cette erreur de wrapper n’affecte pas le run Forgejo live, qui a fini `passed`.
