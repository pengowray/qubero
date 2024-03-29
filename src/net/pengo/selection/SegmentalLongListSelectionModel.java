/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/*
 * SegmentalLongListSelectionModel.java
 *
 * Created on 20 November 2002, 01:34
 */

package net.pengo.selection;


import java.util.ArrayList;
import java.util.Iterator;
import java.util.Set;
import java.util.SortedSet;
import java.util.TreeSet;
import javax.swing.event.EventListenerList;

/**
 *
 * @author  Peter Halasz
 */
public class SegmentalLongListSelectionModel implements LongListSelectionModel {
    
    private static final long MIN = -1;
    private static final long MAX = Long.MAX_VALUE;
    private static final Segment EMPTY_SEGMENT = new Segment(-1, -1);
    
    private int selectionMode = MULTIPLE_INTERVAL_SELECTION;
    //private long minIndex = MAX;
    //private long maxIndex = MIN;
    private long anchorIndex = -1;
    private long leadIndex = -1;
    private long firstAdjustedIndex = MAX;
    private long lastAdjustedIndex = MIN;
    private boolean isAdjusting = false;
    
    //private long firstChangedIndex = MAX;
    //private long lastChangedIndex = MIN;
    
    private TreeSet intervalSet = new TreeSet();
    private TreeSet cacheSet = new TreeSet();
    private boolean isCacheValid = true;
    private boolean isIntervalsetUptodate = true;
    private Segment activeSelection = EMPTY_SEGMENT;
    private Segment activeAntiSelection = EMPTY_SEGMENT;
    private boolean activeSelectionIsAnti = false;
    // should selecting and deselecting destroy the selection behind it? (if done in one action)
    private boolean destructiveSelection = false; //fixme: allow
    
    
    private EventListenerList listenerList = new EventListenerList();
    
    /** Creates a new instance of SegmentalLongListSelectionModel */
    public SegmentalLongListSelectionModel() {
    }
    
    /** copy out data from model */
    public SegmentalLongListSelectionModel(LongListSelectionModel model) {
        for (Segment s : model.getSegments()) {
            addSelectionInterval(s.firstIndex, s.lastIndex);
        }
        
        setLeadSelectionIndex( model.getLeadSelectionIndex() );
        setAnchorSelectionIndex( model.getAnchorSelectionIndex() );
        
    }
    
    // implements javax.swing.ListSelectionModel
    public void addLongListSelectionListener(LongListSelectionListener l) {
	listenerList.add(LongListSelectionListener.class, l);
    }
    
    // implements javax.swing.ListSelectionModel
    public void removeLongListSelectionListener(LongListSelectionListener l) {
	listenerList.remove(LongListSelectionListener.class, l);
    }
    /**
     * Change the selection to be the set union of the current selection
     * and the indices between index0 and index1 inclusive.  If this represents
     * a change to the current selection, then notify each
     * ListSelectionListener. Note that index0 doesn't have to be less
     * than or equal to index1.
     *
     * @param index0 one end of the interval.
     * @param index1 other end of the interval
     * @see #addListSelectionListener
     *
     */
    public void addSelectionInterval(long index0, long index1) {
	if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION) {
	    setSelectionInterval(index0, index1);
	    return;
	}
	
	editIntervalSet();
	Segment change = changeActiveSelectionInterval(index0, index1, false);
	fireValueChanged(change.firstIndex, change.lastIndex);
    }
    
    /**
     * Change the selection to be the set difference of the current selection
     * and the indices between index0 and index1 inclusive.  If this represents
     * a change to the current selection, then notify each
     * ListSelectionListener.  Note that index0 doesn't have to be less
     * than or equal to index1.
     *
     * @param index0 one end of the interval.
     * @param index1 other end of the interval
     * @see #addListSelectionListener
     *
     */
    public void removeSelectionInterval(long index0, long index1) {
	if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION) {
	    //FIXME: can't remove when there's nothing? hmm
	}
	
	editIntervalSet();
	Segment change = changeActiveSelectionInterval(index0, index1, true);
	fireValueChanged(change.firstIndex, change.lastIndex);
    }
    
    
    public TreeSet viewIntervalSet() {
	if (isIntervalsetUptodate)
	    return intervalSet;
	
	if (!isCacheValid) {
	    cacheSet = (TreeSet)intervalSet.clone();
	    fixActiveSelection(cacheSet);
	    isCacheValid = true;
	}
	
	return cacheSet;
    }
    
    private TreeSet editIntervalSet() {
	if (!isIntervalsetUptodate) {
	    if (isCacheValid) {
		intervalSet = (TreeSet)cacheSet.clone(); //FIXME: just assign without clone? may require extra tracking? is clone faster than fixActiveSelection?
	    }
	    else {
		fixActiveSelection(intervalSet);
	    }
	    
	    activeSelection = EMPTY_SEGMENT;
	    activeAntiSelection = EMPTY_SEGMENT;
	    isIntervalsetUptodate = true;
	    isCacheValid = false;
	}
	
	return intervalSet;
    }
    
    
    /*
     * adds the selection interval but does not update lead/anchor, nor fires change event.
     * returns segment over the changed range.
     *
     * called only by viewIntervalSet and editIntervalSet, which check if it should be called
     * and puts whichever treeset into the right state first.
     */
    private void fixActiveSelection(TreeSet tofix) {
	if (activeAntiSelection != EMPTY_SEGMENT) {
	    Segment seg = activeAntiSelection;
	    
	    Segment leftOverlap = findLeftOverlap(seg, false);
	    Segment rightOverlap = findRightOverlap(seg, false);
	    
	    Segment leftCut = null;
	    Segment rightCut = null;
	    
	    tofix.subSet(seg, new Index(seg.lastIndex+1)).clear(); // FIXME: does this work? (remove all)
	    
	    if (leftOverlap != null ) {
		leftCut = new Segment(leftOverlap.firstIndex, seg.firstIndex -1);
		tofix.remove(leftOverlap);
	    }
	    
	    if (rightOverlap != null ) {
		rightCut = new Segment(seg.lastIndex +1, rightOverlap.lastIndex);
		tofix.remove(rightOverlap);
	    }
	    
	    if (leftCut != null) tofix.add(leftCut);
	    if (rightCut != null) tofix.add(rightCut);
	    
	}
	
	
	if (activeSelection != EMPTY_SEGMENT) {
	    //Segment seg = new Segment(index0, index1);
	    Segment seg = activeSelection;
	    Segment stretchedSeg;
	    
	    // find what's there now
	    Segment leftOverlap = findLeftOverlap(seg, true);
	    Segment rightOverlap = findRightOverlap(seg, true);
	    
	    if (leftOverlap != null || rightOverlap != null) {
		long newStart = (leftOverlap == null ? seg.firstIndex : leftOverlap.firstIndex);
		long newEnd = (rightOverlap == null ? seg.lastIndex : rightOverlap.lastIndex);
		stretchedSeg = new Segment(newStart, newEnd);
	    }
	    else {
		stretchedSeg = seg;
	    }
	    
	    tofix.subSet(stretchedSeg, new Index(stretchedSeg.lastIndex+1)).clear(); // FIXME: does this work? (remove all)
	    
	    tofix.add( stretchedSeg );
	}
    }
    
    /* returns a segment over the range that has been changed by the active segment only  */
    private Segment changeActiveSelectionInterval(long index0, long index1, boolean anti) {
	Segment oldActive;
	Segment newActive;
	
	if (!activeSelectionIsAnti) {
	    oldActive = activeSelection;
	}
	else {
	    oldActive = activeAntiSelection;
	}
	
	if (activeSelectionIsAnti != anti) {
	    // changing from anti to non-anti or vice versa, so fix old selection
	    editIntervalSet();
	}
	
	
	if (index0 == -1 || index1 == -1) {
	    deactivateSelectionInterval();
	    return oldActive;
	}
	else if (getSelectionMode() == SINGLE_SELECTION) {
	    newActive = new Segment(index1, index1);
	}
	else {
	    newActive = new Segment(index0, index1);
	}
	
	
	if (!anti) {
	    activeSelection = newActive;
	    activeAntiSelection = EMPTY_SEGMENT;
	    activeSelectionIsAnti = false;
	    
	}
	else {
	    activeSelection = EMPTY_SEGMENT;
	    activeAntiSelection = newActive;
	    activeSelectionIsAnti = true;
	}
	
	anchorIndex = index0;
	leadIndex = index1;
	isCacheValid = false;
	isIntervalsetUptodate = false;
	
	//FIXME: doesn't take into account "background" selection.
	return new Segment(Math.min(oldActive.firstIndex,newActive.firstIndex),
			   Math.max(oldActive.lastIndex,newActive.lastIndex));
    }
    
    private void deactivateSelectionInterval() {
	if (!isIntervalsetUptodate || activeAntiSelection != EMPTY_SEGMENT || activeSelection != EMPTY_SEGMENT)
	    editIntervalSet();
	
	anchorIndex = -1;
	leadIndex = -1;
    }
    
    
    /**
     * Change the selection to be between index0 and index1 inclusive.
     * If this represents a change to the current selection, then
     * notify each ListSelectionListener. Note that index0 doesn't have
     * to be less than or equal to index1.
     *
     * @param index0 one end of the interval.
     * @param index1 other end of the interval
     * @see #addListSelectionListener
     *
     */
    public void setSelectionInterval(long index0, long index1) {
	//FIXME: if we got min+max from elsewhere (eg calculated all the time) then this would be much  more optimised
	long oldMin = getMinSelectionIndex();
	long oldMax = getMaxSelectionIndex();
	
	Segment change = changeActiveSelectionInterval(index0, index1, false);
	
	long minChange = Math.max(change.firstIndex, oldMin);
	long maxChange = Math.max(change.lastIndex, oldMax);
	
	intervalSet.clear();
	isCacheValid = false;
	isIntervalsetUptodate = false;
	
	fireValueChanged(minChange, maxChange);
    }
    
    
    
    
    /**
     * returns the segment (from intervalSet) to the left of seg which overlaps it
     * (or just touches it if allowTouching is true). returns null if none found.
     */
    private Segment findLeftOverlap(Index seg, boolean allowTouching) {
	SortedSet justBeforeSet = intervalSet.headSet(seg);
	if (justBeforeSet.isEmpty())
	    return null;
	
	Segment justBefore = (Segment)justBeforeSet.last();
	
	long adjust = (allowTouching ? 1 : 0);
	
	if (justBefore.lastIndex < seg.firstIndex - adjust) {
	    return null;
	}
	
	return justBefore;
    }
    
    /**
     * returns the segment (from intervalSet) which overlaps it the right edge of seg
     * (or just touches it if allowTouching is true). returns null if none found.
     */
    private Segment findRightOverlap(Segment seg, boolean allowTouching) {
	long adjust = (allowTouching ? 2 : 1);
	
	SortedSet justBeforeSet = intervalSet.headSet(new Index(seg.lastIndex + adjust));
	if (justBeforeSet.isEmpty())
	    return null;
	
	Segment justBefore = (Segment)justBeforeSet.last();
	
	
	
	if (justBefore.lastIndex <= seg.lastIndex) {
	    return null;
	}
	
	return justBefore;
    }
    
    
    /** Change the selection to the empty set.  If this represents
     * a change to the current selection then notify each ListSelectionListener.
     *
     * @see #addListSelectionListener
     *
     */
    public void clearSelection() {
	//FIXME: should this change anchor + lead?
	
	if (intervalSet.isEmpty() && activeSelection == EMPTY_SEGMENT && activeAntiSelection == EMPTY_SEGMENT)
	    return;
	
	long first = getMinSelectionIndex();
	long last = getMaxSelectionIndex();
	
	intervalSet.clear();
	isCacheValid = false;
	isIntervalsetUptodate = true;
	activeSelectionIsAnti = false;
	
	deactivateSelectionInterval();
	activeSelection = EMPTY_SEGMENT;
	activeAntiSelection = EMPTY_SEGMENT;
	anchorIndex = -1;
	leadIndex = -1;
	
	fireValueChanged(first, last);
    }
    
    /** Return the first index argument from the most recent call to
     * setSelectionInterval(), addSelectionInterval() or removeSelectionInterval().
     * The most recent index0 is considered the "anchor" and the most recent
     * index1 is considered the "lead".  Some interfaces display these
     * indices specially, e.g. Windows95 displays the lead index with a
     * dotted yellow outline.
     *
     * @see #getLeadSelectionIndex
     * @see #setSelectionInterval
     * @see #addSelectionInterval
     *
     */
    public long getAnchorSelectionIndex() {
	return anchorIndex;
    }
    
    /** Return the second index argument from the most recent call to
     * setSelectionInterval(), addSelectionInterval() or removeSelectionInterval().
     *
     * @see #getAnchorSelectionIndex
     * @see #setSelectionInterval
     * @see #addSelectionInterval
     *
     */
    public long getLeadSelectionIndex() {
	return leadIndex;
    }
    
    /** Returns the last selected index or -1 if the selection is empty.
     *
     */
    public long getMaxSelectionIndex() {
	/*
	 
	 if (intervalSet.isEmpty() && activeSelection == EMPTY_SEGMENT)
	 return -1;
	 
	 if (intervalSet.isEmpty()) {
	 return activeSelection.lastIndex;
	 }
	 
	 if (activeSelection == EMPTY_SEGMENT) {
	 return ((Segment)intervalSet.last()).lastIndex;
	 }
	 
	 return Math.max(((Segment)intervalSet.last()).lastIndex, activeSelection.lastIndex);
	 */
	
	TreeSet tree = viewIntervalSet();
	if (tree.isEmpty())
	    return -1;
	
	return ((Segment)tree.last()).lastIndex;
	
    }
    
    /** Returns the first selected index or -1 if the selection is empty.
     *
     */
    public long getMinSelectionIndex() {
	//FIXME: should this return -1 or MAX for no selection?
	
	/*
	 //FIXME: perhaps does too much to avoid viewIntervalSet();
	 if (intervalSet.isEmpty() && activeSelection == EMPTY_SEGMENT && activeAntiSelection == EMPTY_SEGMENT)
	 return -1;
	 
	 if (!activeSelectionIsAnti && !destructiveSelection) {
	 if (intervalSet.isEmpty()) {
	 return activeSelection.firstIndex;
	 }
	 
	 if (activeSelection == EMPTY_SEGMENT) {
	 return ((Segment)intervalSet.first()).firstIndex;
	 }
	 
	 return Math.min(((Segment)intervalSet.first()).firstIndex, activeSelection.firstIndex);
	 }
	 */
	
	TreeSet tree = viewIntervalSet();
	if (tree.isEmpty())
	    return MAX;
	
	return ((Segment)tree.first()).firstIndex;
	
    }
    
    // returns the total length of all selections
    public long getSelectionLength() {
	TreeSet tree = viewIntervalSet();
	
	if (tree.isEmpty()) // just for speed
	{
	    return 0;
	}
	
	long length = 0;
	for (Iterator i = tree.iterator(); i.hasNext();) {
	    Segment seg = (Segment)i.next();
	    length += 1 + (seg.lastIndex - seg.firstIndex);
	    
	}
	return length;
    }
    
    public long getSegmentCount() {
	return viewIntervalSet().size();
    }
    
    public Segment[] getSegments() {
	return (Segment[])viewIntervalSet().toArray(new Segment[0]);
    }
    
    //fixme: doesn't scale. array can only be Integer.MAX_VALUE in size.
    public long[] getSelectionIndexes() {
	
	TreeSet tree = viewIntervalSet();
	
	if (tree.isEmpty()) {
	    return new long[0];
	}
	
	long[] array = new long[(int)getSelectionLength()];
	int arraypos = 0;
	
	for (Iterator i = tree.iterator(); i.hasNext();) {
	    Segment seg = (Segment)i.next();
	    for (long ind = seg.firstIndex; ind <= seg.lastIndex; ind++) {
		array[arraypos++] = ind;
	    }
	}
	
	return array;
    }
    
    /** Returns the current selection mode.
     * @return The value of the selectionMode property.
     * @see #setSelectionMode
     *
     */
    public int getSelectionMode() {
	return selectionMode;
    }
    
    /** Returns true if the value is undergoing a series of changes.
     * @return true if the value is currently adjusting
     * @see #setValueIsAdjusting
     *
     */
    public boolean getValueIsAdjusting() {
	return isAdjusting;
    }
    
    /**
     * Insert length indices beginning before/after index.  This is typically
     * called to sync the selection model with a corresponding change
     * in the data model.
     *
     */
    public void insertIndexInterval(long index, long length, boolean before) {
	//editIntervalSet(); // unneeded
	
	/* The first new index will appear at insMinIndex and the last
	 * one will appear at insMaxIndex
	 */
	long insMinIndex = (before) ? index : index + 1;
	long insMaxIndex = (insMinIndex + length) - 1;
	
	// shift everything after the insertion to the right
	SortedSet shiftSet = intervalSet.tailSet(new Index(insMinIndex));
	Set newSet = new TreeSet(); //FIXME: what type should this be optimially?
	Iterator i = shiftSet.iterator();
	while (i.hasNext()) {
	    Segment seg = (Segment)i.next();
	    Segment newSeg = new Segment(seg.firstIndex + length, seg.lastIndex + length);
	    newSet.add(newSeg);
	    
	    i.remove();
	}
	intervalSet.addAll(newSet);
	
	// shift part of the overhanging piece to the right
	Segment snipsnip = findLeftOverlap(new Index(insMinIndex), false);
	if (snipsnip != null) {
	    Segment rightOfSnip = new Segment(insMaxIndex + 1, snipsnip.lastIndex + length);
	    snipsnip.lastIndex = insMinIndex -1;
	}
	
	if (anchorIndex >= index && anchorIndex <= index + length) {
	    anchorIndex += length;
	}
	
	if (leadIndex >= index && leadIndex <= index + length) {
	    leadIndex += length;
	}
	
	activeSelection = moveSegment(activeSelection, index, length);
	activeAntiSelection = moveSegment(activeAntiSelection, index, length);
	
	isCacheValid = false;
	//isIntervalsetUptodate = ... // unchanged
	
	fireValueChanged(insMinIndex, getMaxSelectionIndex());
    }
    
    private Segment moveSegment(Segment seg, long index, long length) {
	if (seg == EMPTY_SEGMENT)
	    return seg;
	
	boolean moveSeg = false;
	long movetoFirst = seg.firstIndex;;
	long movetoLast = seg.lastIndex;;
	
	if (seg.firstIndex >= index) {
	    movetoFirst = seg.firstIndex + length;
	    moveSeg = true;
	}
	
	if (seg.lastIndex >= index) {
	    movetoLast = seg.lastIndex + length;
	    moveSeg = true;
	}
	
	if (moveSeg) {
	    return new Segment(movetoFirst, movetoLast);
	}
	else {
	    return seg;
	}
	
    }
    
    /*
     // unused
     private Segment getActiveSelectionOrAnti() {
     if (activeSelectionIsAnti) {
     return activeAntiSelection;
     } else {
     return activeSelection;
     }
     
     }
     */
    
    /**
     * Remove the indices in the interval index0,index1 (inclusive) from
     * the selection model.  This is typically called to sync the selection
     * model width a corresponding change in the data model.
     *
     */
    public void removeIndexInterval(long index0, long index1) {
	
    }
    
    
    /**
     * Returns true if the specified index is selected.
     *
     */
    public boolean isSelectedIndex(long index) {
	if (index >= activeSelection.firstIndex
	    && index <= activeSelection.lastIndex) {
	    return true;
	}
	
	TreeSet tree = viewIntervalSet();
	
	if (tree.isEmpty()) {
	    return false;
	}
	
	SortedSet headset = tree.headSet(new Index(index+1));
	
	if (headset.isEmpty()) {
	    return false;
	}
	
	if ( ((Segment)headset.last()).lastIndex < index ) {
	    return false;
	}
	
	return true;
	
    }
    
    /** Returns true if no indices are selected.
     *
     */
    public boolean isSelectionEmpty() {
	TreeSet tree = viewIntervalSet();
	return tree.isEmpty();
    }
    
    
    /** Set the anchor selection index.
     *
     * @see #getAnchorSelectionIndex
     *
     */
    public void setAnchorSelectionIndex(long index) {
	Segment change = new Segment(index, anchorIndex);
	
	if (selectionMode == MULTIPLE_INTERVAL_SELECTION) {
	    //FIXME: this can't be right. how to choose between multiple selections and not?
	    
	    editIntervalSet();
	    anchorIndex = index;
	}
	else {
	    //angeActiveSelectionInterval(index, leadIndex, false);
	    //FIXME: clear or something?
	    anchorIndex = index;
	}
	
	//isCacheValid = false; // handled by changeActiveSllection
	//isIntervalsetUptodate = false; // ditto
	
	fireValueChanged(change.firstIndex, change.lastIndex);
    }
    
    /** Set the lead selection index.
     *
     * @see #getLeadSelectionIndex
     *
     */
    public void setLeadSelectionIndex(long index) {
	Segment change = new Segment(index, leadIndex);
	changeActiveSelectionInterval(anchorIndex, index, activeSelectionIsAnti);
	fireValueChanged(change.firstIndex, change.lastIndex);
    }
    
    
    /** Set the selection mode. The following selectionMode values are allowed:
     * <ul>
     * <li> <code>SINGLE_SELECTION</code>
     *   Only one list index can be selected at a time.  In this
     *   mode the setSelectionInterval and addSelectionInterval
     *   methods are equivalent, and only the second index
     *   argument (the "lead index") is used.
     * <li> <code>SINGLE_INTERVAL_SELECTION</code>
     *   One contiguous index interval can be selected at a time.
     *   In this mode setSelectionInterval and addSelectionInterval
     *   are equivalent.
     * <li> <code>MULTIPLE_INTERVAL_SELECTION</code>
     *   In this mode, there's no restriction on what can be selected.
     * </ul>
     *
     * @see #getSelectionMode
     *
     */
    public void setSelectionMode(int selectionMode) {
	if (this.selectionMode == selectionMode) {
	    return;
	}
	Segment oldActiveSelection = activeSelection;
	
	if (this.selectionMode == MULTIPLE_INTERVAL_SELECTION
	    && selectionMode == SINGLE_INTERVAL_SELECTION
	    && intervalSet.size() >= 1) {
	    /*
	     //FIXME: keep only the set which lead or anchor lay on
	     //for now just keep first segment
	     Segment keep = (Segment)intervalSet.first();
	     SortedSet nokeep = intervalSet.tailSet(new Index(keep.firstIndex+1));
	     long firstChange = ((Segment)nokeep.first()).firstIndex;
	     long lastChange = ((Segment)nokeep.last()).lastIndex;
	     intervalSet.removeAll(nokeep);
	     this.selectionMode = selectionMode;
	     fireValueChanged(firstChange, lastChange);
	     */
	    
	    // clear everything but the active selection
	    long first = getMinSelectionIndex();
	    long last = getMaxSelectionIndex();
	    
	    intervalSet.clear(); // deliberately don't clear active selction
	    
	    isCacheValid = false;
	    isIntervalsetUptodate = false;
	    
	    fireValueChanged(first, last); //FIXME: could be calculated better
	    
	}
	else if (selectionMode == SINGLE_SELECTION) {
	    
	    long first = getMinSelectionIndex();
	    long last = getMaxSelectionIndex();
	    leadIndex = anchorIndex;
	    this.selectionMode = selectionMode;
	    clearSelection(); //FIXME: may not fire anything if selection is empty, so no notification about lead change.
	}
	else {
	    this.selectionMode = selectionMode;
	}
	
	
    }
    
    /** This property is true if upcoming changes to the value
     * of the model should be considered a single event. For example
     * if the model is being updated in response to a user drag,
     * the value of the valueIsAdjusting property will be set to true
     * when the drag is initiated and be set to false when
     * the drag is finished.  This property allows listeners to
     * to update only when a change has been finalized, rather
     * than always handling all of the intermediate values.
     *
     * @param valueIsAdjusting The new value of the property.
     * @see #getValueIsAdjusting
     *
     */
    public void setValueIsAdjusting(boolean valueIsAdjusting) {
	if (this.isAdjusting && !valueIsAdjusting) {
	    this.isAdjusting = false;
	    long oldLastAdjustedIndex = lastAdjustedIndex;
	    long oldFirstAdjustedIndex = firstAdjustedIndex;
	    lastAdjustedIndex = MIN;
	    firstAdjustedIndex = MAX;
	    fireValueChanged(oldFirstAdjustedIndex, oldLastAdjustedIndex );
	    return;
	}
	
	this.isAdjusting = valueIsAdjusting;
    }
    
    
    /**
     * @param firstIndex the first index in the interval
     * @param lastIndex the last index in the interval
     * @param isAdjusting true if this is the final change in a series of
     *		adjustments
     * @see EventListenerList
     */
    protected void fireValueChanged(long firstIndex, long lastIndex) {
	Object[] listeners = listenerList.getListenerList();
	boolean isAdjusting = getValueIsAdjusting();
	
	if (isAdjusting) {
	    firstAdjustedIndex = Math.min(firstIndex, firstAdjustedIndex);
	    lastAdjustedIndex = Math.max(lastIndex, lastAdjustedIndex);
	}
	else {
	    firstAdjustedIndex = MAX;
	    lastAdjustedIndex = MIN;
	}
	
	LongListSelectionEvent e = null;
	
	for (int i = listeners.length - 2; i >= 0; i -= 2) {
	    if (listeners[i] == LongListSelectionListener.class) {
		if (e == null) {
		    e = new LongListSelectionEvent(this, firstIndex, lastIndex, isAdjusting);
		}
		((LongListSelectionListener)listeners[i+1]).valueChanged(e);
	    }
	}
	
    }
    
    public Object clone() {
	try {
	    SegmentalLongListSelectionModel clone = (SegmentalLongListSelectionModel)super.clone();
	    clone.intervalSet = (TreeSet)intervalSet.clone();
	    
	    clone.listenerList = new EventListenerList();
	    return clone;
	}
	catch ( CloneNotSupportedException e) {
	    //FIXME: hush hush
	    e.printStackTrace();
	    return null;
	}
	
    }
    
    
    public String toString() {
        long segs = getSegmentCount();
        if (segs==0){
            return "empty";
        } else if (segs == 1){
            long size = getSelectionLength();
            return size + " bytes";
            //return getMinSelectionIndex() +"-"+ getMaxSelectionIndex();
        } else {
            return segs + " segs ";
            //+ size +" length";
        }
        //String s =  ((getValueIsAdjusting()) ? "~" : "=") + value.toString();
        //return getClass().getName() + " " + Integer.toString(hashCode()) + " " + s;
    }
    
	//fixme: general and very inefficient, not to mention ugly.
	public LongListSelectionModel slice(long start, long length) {
    	long[] bits = getSelectionIndexes();
		
		long index = start;
		long remain = length;
			
		if (start < 0) {
			// skip negatives
			index = 0;
			remain = length + start;
		}
		
		if (start >= bits.length)
			// start as after the end
			return new SimpleLongListSelectionModel(-1,-1);
		
		SegmentalLongListSelectionModel ret = new SegmentalLongListSelectionModel();
		
		while (remain > 0 && index < bits.length) {
			ret.addSelectionInterval(bits[(int)index],bits[(int)index]);
			
			index++;
			remain--;
		}
		
		return ret;
	}
    
    
    
}
