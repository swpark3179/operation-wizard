# RAG + Confluence 연동 가이드

이 문서는 Operation Wizard의 **사내 문서 RAG 검색**과 **Confluence 수집** 기능을 **처음부터 실제로
동작**시키기 위한 실무 가이드다. 개념·설계 근거는 `docs/design/`(특히
[05-decisions.md](design/05-decisions.md) D46/D48/D50/D51/D53/D59/D63)에 있고, 이 문서는 **무엇을
채우고, 어디에 무엇을 입력하고, 안 될 때 무엇을 확인하는지**만 다룬다.

> 대상: 이 앱을 사내에서 운영·배포하는 담당자. A절(개발 작업)만 개발 환경이 필요하고, 나머지(B~F)는
> 앱 안에서 클릭만으로 끝난다.

---

## 0. 현재 상태 — 무엇이 되고, 무엇을 채워야 하나

| 구성요소 | 상태 | 위치 |
|----------|------|------|
| Confluence 크롤(수집) 파이프라인 | ✅ **완성** — BFS 재귀 크롤, Bearer PAT 인증, 진행 스트리밍/취소, 연결 테스트 | `src-tauri/src/confluence.rs` |
| 설정 UI(지식 화면) | ✅ **완성** — RAG/Confluence 설정 폼 + 지식 베이스 CRUD | `src/components/KnowledgeView.tsx` |
| 워크플로우 연동(rag 단계) | ✅ **완성** — preflight가 검색 → '검색 결과' 탭 + 프롬프트 주입 | `src/components/ChatPanel.tsx` |
| **RAG 어댑터(ingest/search)** | ❌ **미구현 스텁** — 사내 RAG 서비스 HTTP 계약에 맞춰 채워야 함 | `src-tauri/src/rag.rs` |

**핵심**: 앱은 사내 RAG 서비스에 **원문을 넘기고, 질의하고, 결과를 받는** 배관만 담당한다. **요약·임베딩·
벡터 검색은 사내 RAG 서비스가 소유**한다. 그 배관의 마지막 두 함수(`ingest_page`/`search`)만 아직
비어 있다.

**채우기 전 동작(안전한 degrade — 앱은 절대 죽지 않음):**
- **수집**: 크롤은 정상 진행하지만 각 페이지를 RAG로 넘기는 단계에서 실패 → "임베딩 0건 · 실패 N건".
- **검색(실행 시)**: rag 단계가 **에이전트 턴 없이 건너뜀**("사내 문서 검색 단계를 건너뜁니다…").
- **연결 테스트**: "이 빌드에는 RAG 연동 모듈이 아직 구성되지 않았습니다…" 안내.

### 데이터 흐름 한눈에

```
[수집 시점]  Confluence REST(body.storage HTML)
   → confluence.rs BFS 크롤(루트/스페이스, 최대 2000페이지·깊이 10)
   → RagClient::ingest_page(page)         ← ★ 여기를 채운다 (A절)
   → (사내 RAG 서비스가 요약·임베딩·저장)

[실행 시점]  워크플로우 rag 단계 preflight
   → rag_search(query, topK)
   → RagClient::search(query, k) → RagHit[] ← ★ 여기를 채운다 (A절)
   → 캔버스 '검색 결과' 탭(HTML) + 에이전트 프롬프트에 발췌 주입
```

---

## A. 추가 개발 작업 — RAG 어댑터 채우기 (필수 선행)

RAG를 실제로 쓰려면 **반드시 먼저** 이 작업을 해야 한다. 채울 대상은 딱 두 함수다.

### A-1. 채울 위치

`src-tauri/src/rag.rs`:
- `RagClient::ingest_page`(**96–99행**) — 현재 `Err("RAG ingest 미구현 …")` 반환.
- `RagClient::search`(**123–126행**) — 현재 `Err("RAG search 미구현 …")` 반환.

두 함수 바로 위(**82–95행**, **105–122행**)에 ` ```ignore ` 스켈레톤 주석이 이미 있다. 이 주석의 코드를
그대로 함수 본문으로 옮기고, 사내 서비스 계약에 맞게 조정하면 된다. 주변 인프라(설정 로드,
`reqwest::blocking::Client` 생성, 워커 스레드, `rag_search` 커맨드)는 이미 다 연결돼 있어 **본문 두 개만**
바꾸면 된다.

### A-2. `ingest_page` — 크롤된 페이지 1건을 사내 ingest 엔드포인트로 전송

넘겨받는 데이터(`IngestPage`, camelCase로 직렬화됨):

| 필드 | 타입 | 의미 |
|------|------|------|
| `id` | string | Confluence 페이지 ID |
| `title` | string | 페이지 제목 |
| `url` | string | 절대 webui 링크(검색 결과 출처 표시용, 빈 값일 수 있음) |
| `contentHtml` | string | `body.storage` **원문 HTML**(앱은 요약하지 않음) |

스켈레톤(그대로 쓰면 `POST {endpoint}/ingest` + 헤더 2개 가정):

```rust
pub fn ingest_page(&self, page: &IngestPage) -> Result<(), String> {
    let mut req = self.http.post(format!("{}/ingest", self.cfg.endpoint)).json(page);
    if let Some(key) = self.cfg.secret_key.as_deref() {
        req = req.header("X-Secret-Key", key);
    }
    if let Some(key) = self.cfg.pass_key.as_deref() {
        req = req.header("X-Pass-Key", key);
    }
    let res = req.send().map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("RAG ingest 실패: HTTP {}", res.status()));
    }
    Ok(())
}
```

### A-3. `search` — 질의를 보내고 응답을 `RagHit`로 매핑

돌려줘야 하는 데이터(`RagHit`, `snippet`만 필수):

| 필드 | 타입 | 필수 | 의미 |
|------|------|------|------|
| `title` | string? | 아니오 | 문서 제목 |
| `url` | string? | 아니오 | 출처 링크 |
| `snippet` | string | **예** | 발췌 본문(검색 결과 카드·프롬프트 주입에 사용) |
| `score` | number? | 아니오 | 관련도 점수(표시용) |

스켈레톤(그대로 쓰면 `POST {endpoint}/search`, body `{query, topK}`, 응답 `{hits: [...]}` 가정):

```rust
pub fn search(&self, query: &str, top_k: u32) -> Result<Vec<RagHit>, String> {
    #[derive(serde::Deserialize)]
    struct SearchResponse { hits: Vec<RagHit> }
    let mut req = self
        .http
        .post(format!("{}/search", self.cfg.endpoint))
        .json(&serde_json::json!({ "query": query, "topK": top_k }));
    if let Some(key) = self.cfg.secret_key.as_deref() {
        req = req.header("X-Secret-Key", key);
    }
    if let Some(key) = self.cfg.pass_key.as_deref() {
        req = req.header("X-Pass-Key", key);
    }
    let res: SearchResponse = req
        .send().map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?
        .json().map_err(|e| e.to_string())?;
    Ok(res.hits)
}
```

### A-4. 사내 서비스 계약에 맞춰 조정할 5가지

스켈레톤은 **가장 흔한 REST 형태를 가정**한 것이다. 사내 RAG 서비스 API 명세와 다음을 대조해 조정한다.

1. **엔드포인트 경로**: `/ingest`·`/search`가 다르면 `format!(...)`의 경로를 바꾼다. (엔드포인트 base는
   지식 화면에서 입력한 값이 `self.cfg.endpoint`로 들어온다.)
2. **인증 방식**: 헤더 이름(`X-Secret-Key`/`X-Pass-Key`)이 사내 규약과 다르면 바꾼다. 헤더가 아니라
   Bearer 토큰이면 `req.bearer_auth(key)`, 쿼리 파라미터면 `.query(&[...])`로 대체. 키가 하나뿐이면
   나머지 블록은 지운다. (두 키는 지식 화면 Secret Key/Pass Key 필드값 — D50.)
3. **요청 body 필드명**: `query`/`topK`가 다르면(`q`/`top_k`/`k` 등) `json!({...})` 키를 맞춘다.
4. **응답 JSON 형태**: 최상위가 `{hits: [...]}`가 아니라 배열이 통째로 오거나(`Vec<RagHit>`로 직접
   역직렬화) 필드명이 다르면(`results`/`documents` 등) `SearchResponse` 구조체와 매핑을 조정한다.
   응답 필드 이름이 `RagHit`(`title`/`url`/`snippet`/`score`)와 다르면 중간 DTO 구조체를 만들어
   `RagHit`로 변환한다.
5. **에러 처리**: 사내 서비스가 표준 에러 바디(예 `{message: "..."}`)를 준다면 그 메시지를 추출해
   `Err`로 반환하면 채팅/진행 표시에 사람이 읽을 수 있는 이유가 뜬다.

> **주의**: 구현 후에는 스텁이 남기던 "미구현" 한글 문자열을 반환하면 **안 된다**. 프론트는 그
> 문자열을 보고 "단계 건너뜀"으로 처리하므로, 실제 HTTP 에러를 반환해야 원인이 드러난다.

### A-5. 참고 상수·설정

- **타임아웃**: `HTTP_TIMEOUT = 120초`(`rag.rs` 49행). RAG 검색이 워크플로우 턴 중간에 실행되므로
  느린 사내 백엔드를 위해 넉넉하게 잡혀 있다(D53). 필요하면 조정.
- **TLS**: `RagClient::new`의 `allow_invalid_certs`는 **Confluence 설정의 "TLS 인증서 검증 생략"과 같은
  값을 공유**한다(기본 false). reqwest가 native-tls=schannel로 빌드되어 **Windows 인증서 저장소**를
  신뢰하므로, 사내 TLS 재서명 프록시가 있으면 **그 CA를 Windows 저장소에 설치**하는 것이 정석이다.
  `danger_accept_invalid_certs`(TLS 예외 체크)는 최후 수단(D48).

### A-6. 빌드·테스트

Rust 빌드는 **MSVC 환경 필수**다(Git Bash·순수 PowerShell 금지 — 상세는
[06-build-and-environment.md](design/06-build-and-environment.md)). "Developer PowerShell for VS 2022"
또는 `vcvars64.bat` 초기화 후:

```powershell
# 단위 테스트 (파서·설정은 플랫폼 무관)
cargo test --manifest-path src-tauri\Cargo.toml

# 개발 실행 / 단독 exe 빌드
npm run tauri dev
npm run tauri build -- --no-bundle
```

> `rag.rs`의 기존 테스트 `stubs_return_actionable_unimplemented_errors`는 스텁이 **"미구현"** 문자열을
> 반환한다고 단언한다. 스텁을 실제 구현으로 바꾸면 이 테스트는 실패하므로 **삭제하거나** 새 계약에 맞게
> 갱신한다(예: mock 서버로 200/에러 응답 검증). `empty_endpoint_is_rejected`·`rag_hit_serde_camel_case`는
> 그대로 둔다.

배포는 GitHub Actions `Release` 워크플로우(수동 트리거, 단독 exe — D43,
[06-build-and-environment.md](design/06-build-and-environment.md)).

---

## B. Confluence 수집 설정·실행 (지식 화면 ②)

앱 좌측 내비게이션에서 **지식**(`Library` 아이콘)으로 이동 → "Confluence 수집" 카드.

### B-1. 입력 필드

| 필드 | 설명 |
|------|------|
| **Base URL** | 컨텍스트 경로 포함(예: `https://wiki.example.com/confluence`). REST 호출의 base. |
| **Personal Access Token** | **Server/DC용 Bearer PAT**. **읽기 전용 권장**. (Atlassian **Cloud의 Basic 인증은 v1 범위 밖** — Server/DC PAT만 지원.) |
| **루트 페이지 ID** | 이 페이지의 **하위 트리를 재귀 수집**(예: `123456789`). |
| **스페이스 키** | 루트 페이지 ID를 비우면, 이 스페이스의 **모든 페이지를 평면 수집**(예: `OPS`). |
| **TLS 인증서 검증 생략** | 위험 옵션. 사내 프록시 CA를 Windows 저장소에 설치하는 것이 안전한 해법. |

- 루트 페이지 ID와 스페이스 키는 **둘 중 하나**를 쓴다(루트 우선; 둘 다 비면 시작점 없음 오류).
- 수집 상한(코드 고정, `confluence.rs` 33–36행): **최대 2000페이지 · 깊이 10 · 페이지당 100건씩 조회**.

### B-2. 절차

1. **설정 저장** — 필드 입력 후 "저장". (Base URL을 비우고 저장하면 설정 해제.)
2. **연결 테스트** — "연결 테스트" 버튼으로 `probe_confluence` 실행. 성공하면 루트(또는 스페이스 첫)
   페이지 제목이 `연결됨 — "제목"`으로 뜬다.
3. **수집 시작** — "수집 시작" 버튼. **Confluence와 RAG 설정이 둘 다 저장돼 있어야 활성화**된다
   (RAG 어댑터가 A절에서 구현돼 있어야 실제로 임베딩됨).

### B-3. 진행 관찰·중지

- 진행 패널에 **수집 / 임베딩 / 실패** 카운터와 현재 페이지 티커가 표시된다.
- **다른 화면으로 이동해도 수집은 계속되고**, 지식 화면으로 돌아오면 실시간 현황이 그대로 보인다(모듈
  싱글턴 스토어 — D51). 수집 중에는 내비게이션 '지식' 아이콘에 점(pulse)이 표시된다.
- "중지" 버튼으로 취소(요청 사이에 취소 플래그를 확인 → `end{canceled}`).
- 수집 중에는 Confluence 설정 저장이 비활성(실행 중 크롤은 시작 시점 설정을 사용).
- 페이지별 실패는 **계속 진행**(목록에 최대 5건 표시), 치명적 실패(루트 조회 실패/시작점 없음)는 중단.

---

## C. RAG 검색 설정 (지식 화면 ①)

"RAG 검색 설정" 카드.

| 필드 | 설명 |
|------|------|
| **Endpoint URL** | 사내 RAG 서비스 base URL(예: `https://rag.example.com`). A절의 `self.cfg.endpoint`. |
| **Secret Key** | 요청 헤더로 보낼 값(A-4의 인증 참조). 비우면 미사용. |
| **Pass Key** | 두 번째 헤더 값. 비우면 미사용. |
| **Top K** | 검색 결과 개수(기본 5). |

- **연결 테스트**: `rag_search("연결 테스트", 1)`을 호출한다. **A절을 먼저 구현해야** 초록색
  `연결됨 (N건 응답)`이 뜬다. 미구현이면 "이 빌드에는 RAG 연동 모듈이 아직 구성되지 않았습니다…"가
  표시된다(`foundation.ts::ragUserError`가 개발용 메시지를 사용자용으로 치환).
- **비밀값은 평문 저장**된다(`settings.json`). 로컬 단일 사용자 앱 전제 — **읽기 전용 키 권장**(D50).
- **레거시 주의**: 구버전 설정의 `apiKey` 필드는 무시되고 `secretKey`/`passKey`는 기본 미설정이 된다.
  (마이그레이션 불필요하나 값이 조용히 버려지므로 **재입력** 필요.)

---

## D. 지식 베이스 (지식 화면 ③) — RAG와 다른 것

혼동하기 쉬운데, "지식 베이스" 카드는 **RAG와 별개**다.

- **제목 + 본문**을 단순 저장(임베딩·벡터 검색 아님). knowledge 기반 단계가 실행 시 **프롬프트에 직접
  주입**(전체 16KB 상한)한다.
- 워크플로우 산출물을 저장한 **산출물(artifact) 항목**은 요약 + 첨부 문서 절대경로 인덱스로 주입되고,
  원문 읽기 접근이 부여된다(D59).
- 즉, **RAG = 사내 문서(Confluence) 대량 검색**, **지식 베이스 = 작업 방식/산출물 소량 직접 주입**. 둘은
  기반 3단계에서 각각 rag 단계·knowledge 단계로 쓰인다.

---

## E. 실행 시점 — 워크플로우에서 어떻게 쓰이나

카테고리 작업을 시작하면 기반 3단계(코드베이스 → **RAG 검색** → 지식) 중 **rag 단계**에서:

1. `ChatPanel.stepPreflight`가 `buildRagQuery(런처 프롬프트 + 옵션 답변, 500자 상한)`으로 질의를 만든다.
2. `rag_search(query)` 호출 → 성공 시 결과를 캔버스 **'검색 결과' 탭**(sandbox iframe, `ragResultHtml`)에
   카드로 표시하고, `formatRagContext`로 에이전트 턴 프롬프트에 발췌를 주입한다.
3. **비차단 degrade**: 미설정 → 건너뜀("지식 화면에서 등록"), 0건 → 건너뜀, 에러 → 건너뜀(사용자 문구),
   preflight 중 Stop → 취소. 어느 경우도 대화가 깨지지 않는다(D44).

**카테고리별**:
- `plan`·`query`·`change`: 완전 기반 3단계(코드베이스 + RAG + 지식).
- `guide`(운영 가이드): 코드베이스를 뺀 **RAG + 지식**만(부분 foundation — D63). 사내 문서를 보기 좋게
  시각화하는 것이 이 카테고리의 강점이라, **'검색 결과' 탭**이 특히 유용하다.

상세: [07-workspace-and-runs.md](design/07-workspace-and-runs.md)·[08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md).

---

## F. 문제 해결 (Troubleshooting)

| 증상 | 확인 |
|------|------|
| **검색 단계가 계속 "건너뜀"** | (1) A절의 rag.rs 스텁을 구현하고 재빌드했는가? (2) 지식 화면에 Endpoint를 저장했는가? (3) 연결 테스트 결과는? |
| **연결 테스트가 "RAG 연동 모듈이 구성되지 않았습니다"** | A절 미구현 상태다 — 스텁을 채워야 한다. |
| **수집이 "임베딩 0건 · 실패 N건"** | A절 `ingest_page`가 미구현이거나 엔드포인트/헤더/응답 계약 불일치. RAG 서비스 로그와 A-4 대조. |
| **TLS `BadSignature`/인증서 오류** | 사내 프록시 CA를 **Windows 인증서 저장소에 설치**(정석) 또는 "TLS 인증서 검증 생략" 체크(위험). reqwest=schannel이라 OS 저장소 신뢰(D48·[06](design/06-build-and-environment.md)). |
| **수집 0건/루트 조회 실패** | 루트 페이지 ID 또는 스페이스 키 정확성, PAT 권한(읽기 전용이라도 대상 스페이스 접근 필요), Base URL 컨텍스트 경로. |
| **설정이 초기화된 것 같음** | `settings.json` 파손 시 기본값으로 동작하고 원본을 `settings.json.corrupt`로 보존(D56). |

비밀값 저장 위치: `%APPDATA%\com.shi.operationwizard\settings.json`(RAG 키·Confluence PAT는 평문).

---

## G. 관련 문서

- 설계 근거(결정 로그): [05-decisions.md](design/05-decisions.md) — D46(검색 결과 탭)·D48(지식 뷰/reqwest/
  Confluence 수집/RAG 어댑터)·D50(RagConfig 헤더 3값)·D51(수집 관찰성)·D53(타임아웃)·D59(산출물 지식)·
  D63(guide 부분 foundation).
- 실행 엔진·기반 3단계: [07-workspace-and-runs.md](design/07-workspace-and-runs.md).
- 가이드 플로우·스킬: [08-guided-flows-and-skills.md](design/08-guided-flows-and-skills.md).
- 빌드·사내망·TLS: [06-build-and-environment.md](design/06-build-and-environment.md).
- 프로젝트 전반: [PROJECT_GUIDE.md](PROJECT_GUIDE.md).
