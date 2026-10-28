import { useMemo } from 'react';
import { useUiSettings } from '../context/UiSettingsContext';
import { Spinner } from './Spinner';
import { EmptyState } from './EmptyState';

export interface DataTableProps {
  columns: string[];
  rows: (string | number | null)[][];
  loading?: boolean;
  emptyText?: string;
  maxHeight?: number | string;
  stickyHeader?: boolean;
}

type CellValue = string | number | null;

function isNumericColumn(rows: (string | number | null)[][], index: number): boolean {
  let found = false;
  for (const row of rows) {
    const value = row[index];
    if (value == null) {
      continue;
    }
    if (typeof value !== 'number') {
      return false;
    }
    found = true;
  }
  return found;
}

function formatCell(value: CellValue): string {
  if (value == null) {
    return '—';
  }
  return String(value);
}

export function DataTable({
  columns,
  rows,
  loading = false,
  emptyText = '暂无数据',
  maxHeight,
  stickyHeader = false,
}: DataTableProps) {
  const { tableCenter, tableBorder } = useUiSettings();
  const numericColumns = useMemo(
    () => columns.map((_, index) => isNumericColumn(rows, index)),
    [columns, rows],
  );

  if (loading) {
    return (
      <div className="table-state">
        <Spinner />
        <span className="table-state-text">加载中…</span>
      </div>
    );
  }

  if (!rows.length) {
    return <EmptyState text={emptyText} />;
  }

  const headerClass = stickyHeader ? 'data-table__head sticky' : 'data-table__head';
  const tableClass = [
    'data-table',
    tableCenter ? 'align-center' : 'align-left',
    tableBorder ? 'bordered' : 'borderless',
  ].join(' ');

  return (
    <div className="table-wrap" style={maxHeight ? { maxHeight } : undefined}>
      <table className={tableClass}>
        <thead className={headerClass}>
          <tr>
            {columns.map((name, index) => (
              <th key={index} className={numericColumns[index] ? 'num' : undefined}>
                <span className="col-name">{name}</span>
                <span className="col-index">{index}</span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex} className={rowIndex % 2 === 1 ? 'even' : undefined}>
              {row.map((value, colIndex) => (
                <td key={colIndex} className={numericColumns[colIndex] ? 'num mono' : undefined}>
                  {formatCell(value)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default DataTable;
