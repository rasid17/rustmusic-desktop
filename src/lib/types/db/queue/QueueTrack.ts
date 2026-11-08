export type QueueTrack = {
  queueId: string;
  profilId: number;
  path: string;
  title: string;
  artist?: string;
  album?: string;
  genre?: string;
  duration?: number;
  cover?: string;
  position: number;
};