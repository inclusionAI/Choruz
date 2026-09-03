import { memo } from "react";

import { avatarColor, avatarInitial } from "../../lib/avatar";

type AvatarSize = "normal" | "small" | "tiny";

const SIZE_CLASS: Record<AvatarSize, string> = {
  normal: "avatar",
  small: "avatar small",
  tiny: "avatar tiny",
};

/** Initial-on-colour avatar shared by every surface that shows a principal. */
export const Avatar = memo(function Avatar({
  name,
  size = "normal",
}: {
  name: string;
  size?: AvatarSize;
}) {
  return (
    <div className={SIZE_CLASS[size]} style={{ background: avatarColor(name) }}>
      {avatarInitial(name)}
    </div>
  );
});
