(function () {
  "use strict";

  function activate(buttons, active) {
    buttons.forEach(function (button) { button.classList.toggle("is-active", button === active); });
  }

  function setupKoyfin() {
    var stocks = {
      samsung: { symbol: "005930 · 삼성전자", sector: "KRX · 반도체", price: "78,400", change: "+2.4%", score: "87", read: "Momentum confirmed", positive: true },
      hynix: { symbol: "000660 · SK하이닉스", sector: "KRX · 반도체", price: "214,500", change: "+4.8%", score: "91", read: "Volume breakout", positive: true },
      hyundai: { symbol: "005380 · 현대차", sector: "KRX · 모빌리티", price: "263,000", change: "+0.6%", score: "79", read: "Trend intact", positive: true },
      naver: { symbol: "035420 · NAVER", sector: "KRX · 플랫폼", price: "204,000", change: "−0.4%", score: "72", read: "Neutral consolidation", positive: false },
      kakao: { symbol: "035720 · 카카오", sector: "KRX · 플랫폼", price: "43,250", change: "−2.1%", score: "48", read: "Drawdown risk", positive: false },
      kodex: { symbol: "069500 · KODEX 200", sector: "KRX · 지수형 ETF", price: "41,880", change: "+1.1%", score: "82", read: "Broad trend resumed", positive: true }
    };
    var rows = Array.prototype.slice.call(document.querySelectorAll("[data-k-stock]"));
    function select(row) {
      var stock = stocks[row.getAttribute("data-k-stock")];
      activate(rows, row);
      document.getElementById("k-symbol").textContent = stock.symbol;
      document.getElementById("k-sector").textContent = stock.sector;
      document.getElementById("k-price").textContent = stock.price;
      var change = document.getElementById("k-change");
      change.textContent = stock.change;
      change.className = stock.positive ? "up" : "down";
      document.getElementById("k-score").textContent = stock.score;
      document.getElementById("k-signal-name").textContent = stock.symbol.split(" · ")[0] + " · score " + stock.score;
      document.getElementById("k-model-read").textContent = stock.read;
      document.getElementById("k-score-arc").style.strokeDasharray = (Number(stock.score) * 3.02) + " 302";
    }
    rows.forEach(function (row) {
      row.addEventListener("click", function () { select(row); });
      row.addEventListener("keydown", function (event) { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); select(row); } });
    });
    document.querySelectorAll(".k-ranges button").forEach(function (button) { button.addEventListener("click", function () { activate(Array.prototype.slice.call(button.parentNode.children), button); }); });
  }

  function setupQuartr() {
    var events = {
      samsung: { company: "삼성전자", ticker: "005930", logo: "S", color: "blue", title: "Q2 2026 earnings call", meta: "August 28, 2026 · 09:00 KST · 54 minutes" },
      hynix: { company: "SK하이닉스", ticker: "000660", logo: "H", color: "orange", title: "Q2 2026 earnings call", meta: "August 29, 2026 · 10:30 KST · 48 minutes" },
      naver: { company: "NAVER", ticker: "035420", logo: "N", color: "green", title: "AI strategy briefing", meta: "September 1, 2026 · 14:00 KST · 37 minutes" },
      hyundai: { company: "현대차", ticker: "005380", logo: "H", color: "navy", title: "2026 investor day", meta: "August 31, 2026 · 16:10 KST · 72 minutes" }
    };
    var eventButtons = Array.prototype.slice.call(document.querySelectorAll("[data-q-event]"));
    eventButtons.forEach(function (button) {
      button.addEventListener("click", function () {
        var item = events[button.getAttribute("data-q-event")];
        activate(eventButtons, button);
        document.getElementById("q-company").textContent = item.company;
        document.getElementById("q-ticker").textContent = item.ticker;
        document.getElementById("q-event-title").textContent = item.title;
        document.getElementById("q-event-meta").textContent = item.meta;
        var logo = document.getElementById("q-logo");
        logo.textContent = item.logo;
        logo.className = "q-company-logo " + item.color;
      });
    });
    var tabs = Array.prototype.slice.call(document.querySelectorAll("[data-q-tab]"));
    var panels = Array.prototype.slice.call(document.querySelectorAll("[data-q-panel]"));
    tabs.forEach(function (button) { button.addEventListener("click", function () { activate(tabs, button); panels.forEach(function (panel) { panel.hidden = panel.getAttribute("data-q-panel") !== button.getAttribute("data-q-tab"); }); }); });
    document.querySelectorAll("[data-q-cite]").forEach(function (button) { button.addEventListener("click", function () { var target = document.getElementById(button.getAttribute("data-q-cite")); document.querySelectorAll(".q-transcript>p").forEach(function (paragraph) { paragraph.classList.remove("is-cited"); }); target.classList.add("is-cited"); target.scrollIntoView({ behavior: "smooth", block: "center" }); }); });
    var play = document.querySelector("[data-q-play]");
    play.addEventListener("click", function () { play.textContent = play.textContent === "▶" ? "Ⅱ" : "▶"; });
    var composer = document.querySelector(".q-composer textarea");
    document.querySelectorAll("[data-q-prompt]").forEach(function (button) { button.addEventListener("click", function () { composer.value = button.textContent; composer.focus(); }); });
    document.querySelector(".q-composer").addEventListener("submit", function (event) { event.preventDefault(); if (composer.value.trim()) { composer.value = ""; composer.placeholder = "Mock question added to this research thread"; } });
  }

  function setupFiscal() {
    var metrics = {
      revenue: { title: "Revenue & growth", values: ["₩236T", "₩258T", "₩279T", "₩301T", "₩323T"], heights: [45, 53, 64, 78, 92] },
      profit: { title: "Operating profit", values: ["₩43T", "₩7T", "₩33T", "₩52T", "₩64T"], heights: [67, 18, 49, 76, 92] },
      fcf: { title: "Free cash flow", values: ["₩30T", "₩8T", "₩26T", "₩34T", "₩41T"], heights: [61, 22, 53, 72, 89] }
    };
    var buttons = Array.prototype.slice.call(document.querySelectorAll("[data-f-metric]"));
    var bars = Array.prototype.slice.call(document.querySelectorAll("#f-bar-chart>div"));
    buttons.forEach(function (button) { button.addEventListener("click", function () { var metric = metrics[button.getAttribute("data-f-metric")]; activate(buttons, button); document.getElementById("f-chart-title").textContent = metric.title; bars.forEach(function (bar, index) { bar.querySelector("i").style.setProperty("--h", metric.heights[index] + "%"); bar.querySelector("b").textContent = metric.values[index]; }); }); });
    document.querySelectorAll(".f-periods button").forEach(function (button) { button.addEventListener("click", function () { activate(Array.prototype.slice.call(button.parentNode.children), button); }); });
  }

  function setupTradingView() {
    var symbols = {
      samsung: { name: "삼성전자 · 1D · KRX", score: "87" },
      hynix: { name: "SK하이닉스 · 1D · KRX", score: "91" },
      hyundai: { name: "현대차 · 1D · KRX", score: "79" },
      naver: { name: "NAVER · 1D · KRX", score: "72" }
    };
    var rows = Array.prototype.slice.call(document.querySelectorAll("[data-tv-symbol]"));
    rows.forEach(function (row) { row.addEventListener("click", function () { var symbol = symbols[row.getAttribute("data-tv-symbol")]; activate(rows, row); document.getElementById("tv-name").textContent = symbol.name; document.getElementById("tv-score").textContent = symbol.score; }); });
    document.querySelectorAll(".tv-filters button").forEach(function (button) { button.addEventListener("click", function () { if (button.textContent.indexOf("Add filter") === -1) button.classList.toggle("is-active"); }); });
  }

  function setupOpenBB() {
    var panel = document.querySelector("[data-ob-panel]");
    document.querySelectorAll("[data-ob-catalog]").forEach(function (button) { button.addEventListener("click", function () { panel.hidden = !panel.hidden; }); });
    document.querySelectorAll("[data-ob-close]").forEach(function (button) { button.addEventListener("click", function () { button.closest("[data-ob-widget]").hidden = true; }); });
    var toast = document.querySelector(".ob-toast");
    document.querySelectorAll("[data-ob-add]").forEach(function (button) { button.addEventListener("click", function () { toast.textContent = button.querySelector("b").textContent + " added to canvas"; toast.classList.add("is-visible"); window.setTimeout(function () { toast.classList.remove("is-visible"); }, 1600); }); });
  }

  function setupQuantus() {
    var steps = Array.prototype.slice.call(document.querySelectorAll("[data-qt-step]"));
    var copy = [
      ["Choose the research universe", "분석 범위와 데이터 준비 상태를 먼저 확인합니다."],
      ["Build your signal recipe", "조합할 신호와 가중치를 정하세요. 합계는 자동으로 100%에 맞춰집니다."],
      ["Define validation conditions", "기간, 비용, 비교 기준을 명시한 뒤에만 결과를 생성합니다."],
      ["Record the research limits", "관찰 결과와 함께 데이터 범위, 가정, 미승인 항목을 남깁니다."]
    ];
    function selectStep(index) {
      steps.forEach(function (step, stepIndex) { step.classList.toggle("is-active", stepIndex === index); step.classList.toggle("is-complete", stepIndex < index); step.querySelector("i").textContent = stepIndex < index ? "✓" : String(stepIndex + 1); });
      document.getElementById("qt-step-label").textContent = "STEP " + (index + 1) + " OF 4";
      document.getElementById("qt-step-title").textContent = copy[index][0];
      document.getElementById("qt-step-copy").textContent = copy[index][1];
    }
    steps.forEach(function (step, index) { step.addEventListener("click", function () { selectStep(index); }); });
    document.querySelector("[data-qt-next]").addEventListener("click", function () { selectStep(2); });
    document.querySelectorAll("[data-qt-weight]").forEach(function (range) { range.addEventListener("input", function () { range.parentNode.querySelector("b").textContent = range.value + "%"; }); });
  }

  function setupLinear() {
    var details = {
      hynix: ["SIG-142", "Validate SK하이닉스 volume breakout", "Score moved from 84 to 91 after the latest EOD materialization. Verify that the participation spike is present in immutable source evidence before accepting the state change."],
      kakao: ["SIG-141", "Investigate 카카오 drawdown acceleration", "The drawdown-risk threshold changed state. Confirm the exact observation window and record why the flag is materially different from the prior run."],
      naver: ["SIG-139", "Check NAVER participation mismatch", "Price trend and volume impulse diverged. Review the source window and decide whether the signal should remain unreviewed."],
      samsung: ["SIG-136", "Document 삼성전자 score crossing 85", "Capture the score change while clearly stating that this beta contains price and volume evidence only."],
      kodex: ["SIG-132", "Compare KODEX 200 trend resume", "Add benchmark context to distinguish broad market movement from instrument-specific strength."],
      hyundai: ["SIG-128", "Confirm 현대차 trend remains intact", "Evidence note completed and the item is ready to remain in the reviewed state."]
    };
    var items = Array.prototype.slice.call(document.querySelectorAll("[data-ln-item]"));
    items.forEach(function (item) { item.addEventListener("click", function () { var detail = details[item.getAttribute("data-ln-item")]; activate(items, item); document.getElementById("ln-detail-id").textContent = detail[0]; document.getElementById("ln-detail-title").textContent = detail[1]; document.getElementById("ln-detail-copy").textContent = detail[2]; }); });
    var palette = document.querySelector("[data-ln-palette]");
    var searchButton = document.querySelector("[data-ln-command]");
    function openPalette() { palette.hidden = false; window.setTimeout(function () { palette.querySelector("input").focus(); }, 0); }
    function closePalette() { palette.hidden = true; }
    searchButton.addEventListener("click", openPalette);
    palette.addEventListener("click", function (event) { if (event.target === palette) closePalette(); });
    document.addEventListener("keydown", function (event) { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") { event.preventDefault(); palette.hidden ? openPalette() : closePalette(); } if (event.key === "Escape") closePalette(); });
    document.querySelector(".ln-comment").addEventListener("submit", function (event) { event.preventDefault(); });
  }

  var concept = document.body.getAttribute("data-concept");
  if (concept === "koyfin") setupKoyfin();
  if (concept === "quartr") setupQuartr();
  if (concept === "fiscal") setupFiscal();
  if (concept === "tradingview") setupTradingView();
  if (concept === "openbb") setupOpenBB();
  if (concept === "quantus") setupQuantus();
  if (concept === "linear") setupLinear();
}());
