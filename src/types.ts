export type CommandResult<T = unknown> = {
  ok: boolean;
  message: string;
  data?: T;
};

export type RedisConfig = {
  host: string;
  port: number;
  database: number;
  pattern: string;
};

export type TypesenseConfig = {
  host: string;
  port: number;
  apiKey: string;
  protocol: "http" | "https";
};
