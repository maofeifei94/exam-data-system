/**
 * 画布表格性能演示页（开发用）：http://localhost:5173/bench.html
 *
 * 生成 10 万行 × 12 列数据，用来验证滚轮滑动、选区、复制在超大数据量下依旧流畅。
 */
import ReactDOM from 'react-dom/client';
import { CanvasTable } from './components/CanvasTable';
import './styles.css';

const ROW_COUNT = 100_000;

const COLUMNS = [
  '序号',
  '姓名',
  '身份证号',
  '准考证号',
  '学校',
  '考点',
  '笔试成绩',
  '面试成绩',
  '总成绩',
  '排名',
  '状态',
  '备注',
];

const SURNAMES = ['张', '王', '李', '赵', '刘', '陈', '杨', '黄', '周', '吴'];
const GIVEN = ['伟', '芳', '娜', '敏', '静', '强', '磊', '洋', '艳', '杰'];
const SCHOOLS = [
  '第一中学',
  '第二中学',
  '实验中学',
  '外国语学校',
  '第三中学',
  '师范大学附属中学',
  '育才中学',
  '第八中学',
];
const SITES = ['东区考点', '西区考点', '南湖考点', '北城考点', '中心考点'];
const STATUS = ['正常', '缺考', '违纪'];
const NOTES = [
  '',
  '',
  '',
  '已通过资格审查',
  '需补充材料',
  '享受加分政策，需现场核验原件',
];

function buildRows(count: number): (string | number | null)[][] {
  const rows = new Array<(string | number | null)[]>(count);
  for (let i = 0; i < count; i += 1) {
    const written = Math.round((((i * 37) % 7000) / 100 + 30) * 100) / 100;
    const interview = Math.round((((i * 53) % 5000) / 100 + 40) * 100) / 100;
    rows[i] = [
      i + 1,
      `${SURNAMES[i % SURNAMES.length]}${GIVEN[(i * 7) % GIVEN.length]}`,
      `1101011990${String(100000 + (i % 900000)).slice(-6)}${String(1000 + (i % 9000)).slice(-4)}`,
      `2026${String(i).padStart(7, '0')}`,
      SCHOOLS[i % SCHOOLS.length],
      SITES[(i * 3) % SITES.length],
      written,
      interview,
      Math.round((written * 0.6 + interview * 0.4) * 100) / 100,
      i % 9973,
      STATUS[i % STATUS.length],
      NOTES[i % NOTES.length],
    ];
  }
  return rows;
}

const startedAt = performance.now();
const ROWS = buildRows(ROW_COUNT);
const BUILD_MS = Math.round(performance.now() - startedAt);

function Bench() {
  return (
    <div style={{ padding: 20 }}>
      <h2 style={{ margin: '0 0 6px', fontSize: 18 }}>画布表格 · 10 万行性能演示</h2>
      <p style={{ margin: '0 0 12px', fontSize: 13, color: '#5c636a' }}>
        共 {ROW_COUNT.toLocaleString()} 行 × {COLUMNS.length} 列，数据生成耗时 {BUILD_MS} ms。
        滚轮纵向滑动，Shift + 滚轮横向滑动；点击 / 拖拽选择区域，Ctrl+C 复制选区。
      </p>
      <CanvasTable
        columns={COLUMNS}
        rows={ROWS}
        maxHeight="calc(100vh - 140px)"
        rowHeight={24}
      />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(<Bench />);
