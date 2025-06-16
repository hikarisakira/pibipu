// TypeScript 版本的 Mongoose 模型，用於管理 YouTube 頻道通知設定
import mongoose, { Document, Model } from "mongoose";

// 定義文檔接口
interface INotificationConfig {
  youtubeChannelId: string;
  youtubeChannelName: string;
  discordGuildId: string;
  discordChannelId: string;
  customMessage?: string;
  lastCheckedVideoId?: string;
  lastChecked: Date;
  createdBy: string;
  isActive: boolean;
  createdAt: Date;
  updatedAt: Date;
}

// 定義文檔類型（包含 Mongoose 方法）
interface INotificationConfigDocument extends INotificationConfig, Document {}

// 定義模型靜態方法接口
interface INotificationConfigModel extends Model<INotificationConfigDocument> {
  cleanupExpiredConfigs(): Promise<void>;
}

const notificationConfigSchema = new mongoose.Schema(
  {
    // YT頻道ID
    youtubeChannelId: {
      type: String,
      required: true,
      index: true,
    },
    // YT頻道名稱
    youtubeChannelName: {
      type: String,
      required: true,
    },
    // Discord伺服器ID
    discordGuildId: {
      type: String,
      required: true,
      index: true,
    },
    // Discord目標通知頻道ID
    discordChannelId: {
      type: String,
      required: true,
    },
    // 自訂訊息
    customMessage: {
      type: String,
      default: null,
    },
    // 最後一次檢查的影片ID
    lastCheckedVideoId: {
      type: String,
      default: null,
    },

    // 最後檢查時間
    lastChecked: {
      type: Date,
      default: Date.now,
    },

    // 建立者
    createdBy: {
      type: String,
      required: true,
    },

    // 是否啟用
    isActive: {
      type: Boolean,
      default: true,
    },
  },
  {
    timestamps: true, // 自動添加 createdAt 和 updatedAt 欄位
  },
);

// 建立複合索引 - 允許同一 YouTube 頻道在同一伺服器的不同頻道設定通知
notificationConfigSchema.index(
  { youtubeChannelId: 1, discordGuildId: 1, discordChannelId: 1 },
  { unique: true },
);

// 清理過期設定
notificationConfigSchema.statics.cleanupExpiredConfigs = async function () {
  const expirationTime = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000); // 30天前
  try {
    const result = await this.deleteMany({
      isActive: false,
      updatedAt: { $lt: expirationTime },
    });

    if (result.deletedCount > 0) {
      console.log(`清理了 ${result.deletedCount} 條過期的通知設定。`);
    }
  } catch (error) {
    console.error("Error during cleanup:", error);
  }
};

export default mongoose.model<
  INotificationConfigDocument,
  INotificationConfigModel
>("NotificationConfig", notificationConfigSchema);
// 將模型導出以供其他模組使用
