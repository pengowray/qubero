package net.pengo.hexdraw.layout;

/*
 * Created on Feb 4, 2005
 *
 package net.pengo.hexdraw.layout;

 /**
 * @author Peter Halasz
 */
public class LayoutCursorDescender {
	private LayoutCursor lc;

	private int depth = 0;

	// is the sescender still in an active area. active means the selection was
	// made in that column
	boolean active = true;
	
	LayoutCursorDescender(LayoutCursor lc) {
		this.lc = lc;
	}
	
	private static LayoutCursorDescender unactiveDescender;
	public static LayoutCursorDescender unactiveDescender() {
		if (unactiveDescender == null){
			unactiveDescender = new LayoutCursorDescender(LayoutCursor.unactiveCursor()) {
				public Object pathItem() { return null; };
				public LayoutCursorDescender descend(Object pathItemCompare) {
					return this;
				}
				public LayoutCursorDescender clone() { return this; }
				public boolean isActive() { return false; }
				public void setActive(boolean active) { return; }
			};
		}
		
		return unactiveDescender;
	}
	
	public Object pathItem() {
		if (depth >= lc.getPath().length)
			return null;
		
		return lc.getPath()[depth];
	}

	public LayoutCursorDescender descend(Object pathItemCompare) {
		LayoutCursorDescender clone = clone();
		if (clone.active == true 
				&& pathItemCompare != null 
				&& clone.pathItem() != null
				&& clone.pathItem().equals(pathItemCompare)) {
			// nothing
		} else {
			clone.active = false;
		}

		clone.depth++;

		return clone;
	}

	public LayoutCursorDescender clone() {
		LayoutCursorDescender clone = new LayoutCursorDescender(lc);
		clone.depth = this.depth;
		clone.active = this.active;
		return clone;
	}

	public boolean isActive() {
		return active;
	}

	public void setActive(boolean active) {
		this.active = active;
	}
	
}
