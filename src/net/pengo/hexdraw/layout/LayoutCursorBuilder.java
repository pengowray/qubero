/*
 * Created on 3/02/2005
 */
package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;

import net.pengo.bitSelection.BitCursor;

/**
 * Temporary to feed down a heirarchy, so as to build a LayoutCursor at the end.
 * 
 * Includes extra "working" information, like an click's X and Y, blink's X and Y,
 * and total bits.
 * 
 * Also serves to differentiate when an arguement is 
 * a constructed LayoutCursor or one still being built. (i.e. self-documentation)
 * 
 * @author Peter Halasz
 */
public class LayoutCursorBuilder extends LayoutCursor implements Cloneable {

	private long blinkX, blinkY; // calculating cursor blink while initializing.. not kept for toLayoutCursor
	private long clickX, clickY; // stores click location.. not kept for toLayoutCursor
	
	private int gTransX, gTransY; // how much we've translated the graphics context
	
	protected BitCursor bits; // total bits (or bits remaining if via a View)
	
	private boolean isNull;

	public void setNull(boolean isNull) {
		this.isNull = isNull;
	}
	
	public LayoutCursor toLayoutCursor() {
		if (isNull)
			return LayoutCursor.unactiveCursor();
		
		LayoutCursor lc = new LayoutCursor();
		lc.setPath(getPath());
		lc.setBitLocation(bitLocation);
		lc.setBlinkyLocation(blinkyLocation);
		
		return lc;
	}

	public void setTranslationPoint(Graphics g, int x, int y) {
		int deltaX = x - gTransX;
		int deltaY = y - gTransY;
		g.translate(deltaX, deltaY);
		gTransX = x;
		gTransY = y;
	}

	public void resetTranslationPoint(Graphics g) {
		g.translate(-gTransX, -gTransY);
		gTransX = 0;
		gTransY = 0;
	}

	
	
	/** 
	 * Translates the coordinate system, so +x +y is the new 0,0.
	 * Can be restored with restorePerspective. */
	public LayoutCursorBuilder perspective(int x, int y) {
		return new LayoutCursorBuilderView(this,x,y,null);
	}
	
	public LayoutCursorBuilder perspective(BitCursor c) {
		return new LayoutCursorBuilderView(this,0,0,c);
	}

	public LayoutCursorBuilder perspective(int x, int y, BitCursor c) {
		return new LayoutCursorBuilderView(this,x,y,c);
	}
	
	public LayoutCursorBuilder restorePerspective() {
		return this;
	}
	
	public LayoutCursorBuilder clone(){
		try {
			//FIXME: do some deeper cloning, maybe, but not neccessary
			return (LayoutCursorBuilder)super.clone();
		} catch (CloneNotSupportedException e) {
			assert false; // shouldn't happen or some such
			e.printStackTrace();
			return null;
		}
	}

	public void setBitLocation(BitCursor bitLocation) {
		this.bitLocation = bitLocation;
	}
	
	public void addToBitLocation(BitCursor addBits) {
		if (bitLocation == null) {
			bitLocation = addBits;
			return;
		}
		
		bitLocation = bitLocation.add(addBits);
	}

	public void addToPath(Object pathObj) {
		getPathList().add(pathObj);
	}
	
	// Don't need this method.
	public LayoutCursorDescender descender() {
		assert false; // shouldn't use this method from this class?
		return new LayoutCursorDescender(toLayoutCursor());
	}
	
	public String toString() {
		String r = "LayoutCursorBuilder loc:" + bitLocation 
		+ " bits=" + bits + " (" + getBits() + ")"
		+ " click X,Y=" + clickX + "," + clickY + " (" + getClickX() + "," + getClickY() + ")" 
		+ " blink X,Y=" + blinkX + "," + blinkY + " (" + getBlinkX() + "," + getBlinkY() + ")" 
		+ " blinky=" + getBlinkyLocation();
		
		
		//+ " path(" + getPath().length + ")";
	
		//for (Object o : getPath()) {
		//	r += " " + o;
		//}
		
		return r;
	}
	
	public long getClickX() {
		return clickX;
	}
	public void setClickX(long x) {
		this.clickX = x;
	}
	public void addToClickX(long add) {
		this.clickX += add;
	}
	public long getClickY() {
		return clickY;
	}
	public void setClickY(long y) {
		this.clickY = y;
	}
	public void addToClickY(long add) {
		this.clickY += add;
	}
	
	
	public long getBlinkX() {
		return blinkX;
	}
	public void setBlinkX(long x) {
		this.blinkX = x;
	}
	public void addToBlinkX(long add) {
		this.blinkX += add;
	}
	public long getBlinkY() {
		return blinkY;
	}
	public void setBlinkY(long y) {
		this.blinkY = y;
	}
	public void addToBlinkY(long add) {
		this.blinkY += add;
	}
	
	public Point getBlinkPoint() {
		return new Point((int)getBlinkX(), (int)getBlinkY());
	}
	
	public LayoutCursorBuilder toBuilder() {
		return this;
	}
	public BitCursor getBits() {
		return bits;
	}
	public void setBits(BitCursor bits) {
		this.bits = bits;
	}

}
 