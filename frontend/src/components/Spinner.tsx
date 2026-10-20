export interface SpinnerProps {
  size?: number;
}

export function Spinner({ size = 22 }: SpinnerProps) {
  return (
    <span
      className="spinner"
      style={{ width: size, height: size }}
      role="status"
      aria-label="加载中"
    />
  );
}

export default Spinner;
