/*
 * Created on Feb 11, 2005
 */
package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;
import java.awt.Rectangle;
import java.util.List;

import net.pengo.bitSelection.BitCursor;


/**
 * @author Peter Halasz
 */
public class LayoutCursorBuilderView extends LayoutCursorBuilder {
	private LayoutCursorBuilder parent;
	private int xTransform; // how much this view shifts from the parent
	private int yTransform;
	
	private BitCursor bTransform; // how mcuh bits have been transformed

	/** bTrans may be null for none */
	public LayoutCursorBuilderView(LayoutCursorBuilder lc, int x, int y, BitCursor bTrans) {
		assert (! (lc instanceof LayoutCursorBuilder)); // should have called .perspective() instead
		
		parent = lc;
		this.xTransform = x;
		this.yTransform = y;
		this.bTransform = bTrans;
	}
	
	public void translateHere(Graphics g) {
		parent.setTranslationPoint(g, xTransform, yTransform);
	}
	
	public Point getTransform() {
		return new Point(xTransform, yTransform);
	}
	
	public LayoutCursorBuilder perspective(int x, int y) {
		return new LayoutCursorBuilderView(parent, xTransform+x, yTransform+y, bTransform);
	}

	public LayoutCursorBuilderView clone(){
			//FIXME: do some deeper cloning, maybe, but not neccessary
			LayoutCursorBuilderView r = (LayoutCursorBuilderView)super.clone();
			r.parent = parent.clone();
			return r;
	}
	
	public LayoutCursorBuilder perspective(BitCursor c) {
		if (bTransform == null)
			return new LayoutCursorBuilderView(parent, xTransform, yTransform, c);
		
		return new LayoutCursorBuilderView(parent, xTransform, yTransform, bTransform.add(c));
	}	
	
	public LayoutCursorBuilder perspective(int x, int y, BitCursor c) {
		if (c == null)
			return perspective(x,y);

		if (bTransform == null)
			return new LayoutCursorBuilderView(parent, xTransform+x, yTransform+y, c);		
		
		return new LayoutCursorBuilderView(parent, xTransform+x, yTransform+y, bTransform.add(c));		
	}

		
	public LayoutCursorBuilder restorePerspective() {
		return parent;
	}
	
	// ***** Translated methods *****
	
	public long getBlinkX() {
		return parent.getBlinkX()-xTransform;
	}
	public long getBlinkY() {
		return parent.getBlinkY()-yTransform;
	}
	public long getClickX() {
		return parent.getClickX()-xTransform;
	}
	public long getClickY() {
		return parent.getClickY()-yTransform;
	}
	public void setBlinkX(long x) {
		parent.setBlinkX(x+xTransform);
	}
	public void setBlinkY(long y) {
		parent.setBlinkY(y+yTransform);
	}
	public void setClickX(long x) {
		parent.setClickX(x+xTransform);
	}
	public void setClickY(long y) {
		parent.setClickY(y+yTransform);
	}
	public BitCursor getBitLocation() {
		if (bTransform == null)
			return parent.getBitLocation();
		
		return parent.getBitLocation().subtract(bTransform);
	}
	public void setBitLocation(BitCursor bitLocation) {
		if (bTransform == null) {
			parent.setBitLocation(bitLocation);
			return;
		}

		parent.setBitLocation(bitLocation.add(bTransform));
	}
	public BitCursor getBits() {
		if (bTransform == null)
			return parent.getBits();
		
		return parent.getBits().subtract(bTransform);
	}
	public void setBits(BitCursor bits) {
		if (bTransform == null) {
			parent.setBits(bits);
			return;
		}

		parent.setBits(bits.add(bTransform));
	}
	
	/** shouldn't really be used in this method from here. */
	public void setTranslationPoint(Graphics g, int x, int y) {
		//FIXME: should probably be protected (same as parent)
		assert false; // don't use this method from here please
		parent.setTranslationPoint(g, x+xTransform, y+yTransform);
	}
	
	// ***** Wrapped methods *****  

	public void addToBlinkX(long add) {
		parent.addToBlinkX(add);
	}
	public void addToBlinkY(long add) {
		parent.addToBlinkY(add);
	}
	public void addToClickX(long add) {
		parent.addToClickX(add);
	}
	public void addToClickY(long add) {
		parent.addToClickY(add);
	}

	public void addToBitLocation(BitCursor addBits) {
		parent.addToBitLocation(addBits);
	}
	
	public void addToPath(Object pathObj) {
		parent.addToPath(pathObj);
	}
	public LayoutCursorDescender descender() {
		return parent.descender();
	}
	public boolean equals(Object obj) {
		return parent.equals(obj);
	}
	public Rectangle getBlinkyLocation() {
		return parent.getBlinkyLocation();
	}
	public Object[] getPath() {
		return parent.getPath();
	}
	public List getPathList() {
		return parent.getPathList();
	}
	public void resetTranslationPoint(Graphics g) {
		parent.resetTranslationPoint(g);
	}
	public void setBlinkyLocation(Rectangle blinkyLocation) {
		parent.setBlinkyLocation(blinkyLocation);
	}
	public void setNull(boolean isNull) {
		parent.setNull(isNull);
	}
	public void setPath(Object[] path) {
		parent.setPath(path);
	}
	public LayoutCursor toLayoutCursor() {
		return parent.toLayoutCursor();
	}
	public String toString() {
		return parent.toString() + " VIEW transform:" + xTransform + "," + yTransform + " bits:" + bTransform;
	}
}
