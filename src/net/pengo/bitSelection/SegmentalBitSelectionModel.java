/*
 * SegmentalLongListSelectionModel.java
 *
 * Created on 20 November 2002, 01:34
 */

package net.pengo.bitSelection;


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
public class SegmentalBitSelectionModel { // implements BitSelectionModel {
    

    int SINGLE_SELECTION = 0;
    int SINGLE_INTERVAL_SELECTION = 1;
    int MULTIPLE_INTERVAL_SELECTION = 2; // please dont change from MULTIPLE_INTERVAL_SELECTION
    
    //private static final long MIN = -1;
    //private static final long MAX = Long.MAX_VALUE;
    
    //private static final BitSegment EMPTY_SEGMENT = null;

    private int selectionMode = MULTIPLE_INTERVAL_SELECTION;
    private EventListenerList listenerList = new EventListenerList();

    private static final DirectionalSegment EMPTY_SEGMENT = new DirectionalSegment(0, 0);

    private TreeSet<BitSegment> intervalSet = new TreeSet(); // contains the main selection data
    private TreeSet<BitSegment> cacheSet = new TreeSet();
    private boolean isCacheValid = true;
    private boolean isIntervalsetUptodate = true;

    private DirectionalSegment activeSelection = EMPTY_SEGMENT;
    private DirectionalSegment activeAntiSelection = EMPTY_SEGMENT;
    private boolean activeSelectionIsAnti = false;
    
    private BitSegment adjustedSegment = EMPTY_SEGMENT; // replaces firstAdjustedIndex + lastAdjustedIndex
    private boolean isAdjusting = false;
    

    //private long minIndex = MAX;
    //private long maxIndex = MIN;
    
    //private BitCursor anchorIndex = null;
    //private BitCursor leadIndex = null;
    //private DirectionalSegment anchorLeadInterval = null;
    
    //private long firstAdjustedIndex = MAX;
    //private long lastAdjustedIndex = MIN;
    
    
    //private long firstChangedIndex = MAX;
    //private long lastChangedIndex = MIN;
    
    
    
    
    //cacheSet holds the intervalSet with the current selection fixed. access it via: previewSaveActiveSelection()
    // should selecting and deselecting destroy the selection behind it? (if done in one action)
    
    //private boolean destructiveSelection = false; //fixme: allow
    
    
    
    /** Creates a new instance of SegmentalLongListSelectionModel */
    public SegmentalBitSelectionModel() {
    }
    
    /** copy out data from model. loses lead/anchor data */
    public SegmentalBitSelectionModel(BitSelectionModel model) {
        for (BitSegment s : model.getSegments()) {
            addSelectionInterval(s);
        }
        
        //fixme: take lead and anchor too. note above fixes model's active selection in the new object 
        
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
    public void addSelectionInterval(BitSegment interval) {
        addSelectionInterval(new DirectionalSegment(interval.firstIndex, interval.lastIndex));
    }

    public void addSelectionInterval(DirectionalSegment interval) {
	if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION) {
	    setSelectionInterval(interval);
	    return;
	}
	
	saveActiveSelection();
	BitSegment change = changeActiveSelectionInterval(interval, false);
	fireValueChanged(change);
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
    public void removeSelectionInterval(DirectionalSegment interval) {
	if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION) {
	    //FIXME: can't remove when there's nothing? hmm
	}
	
	saveActiveSelection();
	BitSegment change = changeActiveSelectionInterval(interval, true);
	fireValueChanged(change);
    }
    
    
    // implements javax.swing.ListSelectionModel
    public void addBitSelectionListener(BitSelectionListener l) {
	listenerList.add(BitSelectionListener.class, l);
    }
    
    // implements javax.swing.ListSelectionModel
    public void removeBitSelectionListener(BitSelectionListener l) {
	listenerList.remove(BitSelectionListener.class, l);
    }
    
    /** returns the interval selection tree containing active selection, 
     * but does not save the active selection to the intervalSet
     */
    public TreeSet<BitSegment> previewSaveActiveSelection() {
	if (isIntervalsetUptodate)
	    return intervalSet;
	
	if (!isCacheValid) {
	    cacheSet = (TreeSet)intervalSet.clone();
	    mergeInSelection(cacheSet);
	    isCacheValid = true;
	}
	
	return cacheSet;
    }
    
    private TreeSet<BitSegment> saveActiveSelection() {
	if (!isIntervalsetUptodate) {
	    if (isCacheValid) {
		intervalSet = (TreeSet)cacheSet.clone(); //FIXME: just assign without clone? may require extra tracking? is clone faster than fixActiveSelection?
	    }
	    else {
		mergeInSelection(intervalSet);
	    }
	    
	    activeSelection = EMPTY_SEGMENT;
	    activeAntiSelection = EMPTY_SEGMENT;
            activeSelectionIsAnti = false; // fixme: should be here? if not, add to setAnchor
	    isIntervalsetUptodate = true;
	    isCacheValid = false;
	}
	
	return intervalSet;
    }
    
    
    /*
     * adds the selection interval but does not update lead/anchor, nor fires change event.
     * returns segment over the changed range.
     *
     * called only by previewSaveActiveSelection and saveActiveSelection, which check if it should be called
     * and puts whichever treeset into the right state first.
     */
    private void mergeInSelection(TreeSet tofix) {
	if (activeAntiSelection != EMPTY_SEGMENT) {
	    BitSegment seg = activeAntiSelection;
	    
	    BitSegment leftOverlap = findLeftOverlap(seg, false);
	    BitSegment rightOverlap = findRightOverlap(seg, false);
	    
	    BitSegment leftCut = null;
	    BitSegment rightCut = null;
	    
	    tofix.subSet(seg.firstIndex, seg.lastIndex).clear(); 
	    
	    if (leftOverlap != null ) {
		leftCut = new BitSegment(leftOverlap.firstIndex, seg.firstIndex);
		tofix.remove(leftOverlap);
	    }
	    
	    if (rightOverlap != null ) {
		rightCut = new BitSegment(seg.lastIndex, rightOverlap.lastIndex);
		tofix.remove(rightOverlap);
	    }
	    
	    if (leftCut != null) tofix.add(leftCut);
	    if (rightCut != null) tofix.add(rightCut);
	    
	}
	
	
	if (activeSelection != EMPTY_SEGMENT) {
	    //Segment seg = new Segment(index0, index1);
	    BitSegment seg = activeSelection;
	    BitSegment stretchedSeg;
	    
	    // find what's there now
	    BitSegment leftOverlap = findLeftOverlap(seg, true);
	    BitSegment rightOverlap = findRightOverlap(seg, true);
	    
	    if (leftOverlap != null || rightOverlap != null) {
		BitCursor newStart = (leftOverlap == null ? seg.firstIndex : leftOverlap.firstIndex);
		BitCursor newEnd = (rightOverlap == null ? seg.lastIndex : rightOverlap.lastIndex);
		stretchedSeg = new BitSegment(newStart, newEnd);
	    }
	    else {
		stretchedSeg = seg;
	    }
	    
	    tofix.subSet(stretchedSeg, stretchedSeg.lastIndex).clear(); 
	    
	    tofix.add( stretchedSeg );
	}
    }
    
    /* returns a segment over the range that has been changed by the active segment only  */
    private BitSegment changeActiveSelectionInterval(DirectionalSegment interval, boolean anti) {
	BitSegment oldActive;
	DirectionalSegment newActive;
	
	if (activeSelectionIsAnti) {
	    oldActive = activeAntiSelection;
	}
	else {
            oldActive = activeSelection;
	}
	
	if (activeSelectionIsAnti != anti) {
	    // changing from anti to non-anti or vice versa, so force fix old selection
	    saveActiveSelection();
	}
	
	if (interval == null) {
	    saveActiveSelection();
	    return oldActive;
	} 
        
        if (getSelectionMode() == SINGLE_SELECTION) {
            //FIXME/NYI: not really supported or tested
            //i think i need to add one byte to index1
	    newActive = new DirectionalSegment(interval.getHead(), interval.getHead().addBits(8));
	} else {
	    newActive = interval;
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
	
	//anchorIndex = index0;
	//leadIndex = index1;
        
	isCacheValid = false;
	isIntervalsetUptodate = false;
	
	//FIXME: doesn't take into account "background"(??) selection.
        
        //return area of change
        BitCursor[] everything = new BitCursor[] { 
            oldActive.firstIndex, newActive.firstIndex,
            oldActive.lastIndex,  newActive.lastIndex
        };
        
	return new BitSegment(everything);
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
    public void setSelectionInterval(DirectionalSegment interval) {
	BitSegment change = changeActiveSelectionInterval(interval, false);
	
	intervalSet.clear();
	isCacheValid = false;
	isIntervalsetUptodate = false;
	
	fireValueChanged(change);
    }
    
    
    
    
    /**
     * returns the segment (from intervalSet) to the left of seg which overlaps it
     * (or just touches it if allowTouching is true). returns null if none found.
     */
    private BitSegment findLeftOverlap(BitSegment objective, boolean allowTouching) {
	
        SortedSet justBeforeSet = intervalSet.headSet(objective);
	if (justBeforeSet.isEmpty())
	    return null;
	BitSegment justBefore = (BitSegment)justBeforeSet.last();
        
        int adjust = (allowTouching ? -1 : 0);
	
        BitCursor objectiveLeft = objective.firstIndex.addBits(adjust);

	if (justBefore.lastIndex.compareTo(objectiveLeft) < 0)
	    return null;
        
	if (justBefore.lastIndex.compareTo(objective.firstIndex.addBits(adjust)) < 0) {
	    return null;
	}
	
	return justBefore;
    }
    
    /**
     * returns the segment (from intervalSet) which overlaps it the right edge of seg
     * (or just touches it if allowTouching is true). returns null if none found.
     */
    private BitSegment findRightOverlap(BitSegment objective, boolean allowTouching) {
	int adjust = (allowTouching ? 2 : 1);
        BitCursor comparePoint = objective.lastIndex.addBits(adjust);
	BitSegment compare = new BitSegment(comparePoint, comparePoint);
	SortedSet justBeforeSet = intervalSet.headSet(compare);
	if (justBeforeSet.isEmpty())
	    return null;
	
	BitSegment justBefore = (BitSegment)justBeforeSet.last();
	
	if (justBefore.lastIndex.equals(new BitCursor())){
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
	if (intervalSet.isEmpty() && activeSelection == EMPTY_SEGMENT && activeAntiSelection == EMPTY_SEGMENT)
	    return;
	
        BitSegment old = new BitSegment(getMinSelectionIndex(), getMaxSelectionIndex());
        
	intervalSet.clear();
	isCacheValid = false;
	isIntervalsetUptodate = true;
	activeSelectionIsAnti = false;
	
	//saveActiveSelection(); // why?
	activeSelection = EMPTY_SEGMENT;
	activeAntiSelection = EMPTY_SEGMENT;
	
	fireValueChanged(old);
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
    public BitCursor getAnchorSelectionIndex() {
        if (!activeSelectionIsAnti)
            return activeSelection.getTail();
        
        return activeAntiSelection.getTail();
    }
    
    /** Return the second index argument from the most recent call to
     * setSelectionInterval(), addSelectionInterval() or removeSelectionInterval().
     *
     * @see #getAnchorSelectionIndex
     * @see #setSelectionInterval
     * @see #addSelectionInterval
     *
     */
    public BitCursor getLeadSelectionIndex() {
        if (!activeSelectionIsAnti)
            return activeAntiSelection.getHead();
        
        return activeSelection.getHead();
    }
    
    /** Returns the last selected index or -1 if the selection is empty.
     *
     */
    public BitCursor getMaxSelectionIndex() {
	
	TreeSet<BitSegment> tree = previewSaveActiveSelection();
	if (tree.isEmpty())
	    return null;
	
	return tree.last().lastIndex;
	
    }
    
    /** Returns the first selected index or -1 if the selection is empty.
     *
     */
    public BitCursor getMinSelectionIndex() {
	//FIXME: should this return -1 or MAX for no selection?
	
	/*
	 //FIXME: perhaps does too much to avoid previewSaveActiveSelection();
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
	
	TreeSet tree = previewSaveActiveSelection();
	if (tree.isEmpty())
            return null;
	    //return MAX;
	
	return ((BitSegment) tree.first()).firstIndex;
	
    }
    
    // returns the total length of all selections
    public BitCursor getSelectionLength() {
	TreeSet tree = previewSaveActiveSelection();
	
	if (tree.isEmpty()) // just for speed
	    return new BitCursor();
	
	BitCursor length = new BitCursor();
	for (Iterator i = tree.iterator(); i.hasNext();) {
	    BitSegment seg = (BitSegment)i.next();
	    length = length.add( seg.getLength() );
	    
	}
	return length;
    }
    
    public long getSegmentCount() {
	return previewSaveActiveSelection().size();
    }
    
    public BitSegment[] getSegments() {
        //(BitSegment[])
	return previewSaveActiveSelection().toArray(new BitSegment[0]);
    }
    
    // NYI
    // doesn't really work well with bits.. wasn't great with bytes either
    public long[] getSelectionIndexes() {
        /*
        //fixme: doesn't scale. array can "only" be Integer.MAX_VALUE in size.
	
	TreeSet tree = previewSaveActiveSelection();
	
	if (tree.isEmpty()) {
	    return new BitSegment[0];
	}
	
	long[] array = new long[(int)getSelectionLength()];
	int arraypos = 0;
	
	for (Iterator i = tree.iterator(); i.hasNext();) {
	    BitSegment seg = (BitSegment)i.next();
	    for (long ind = seg.firstIndex; ind <= seg.lastIndex; ind++) {
		array[arraypos++] = ind;
	    }
	}
	
	return array;
         */
        
        return new long[0];
       
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
	//saveActiveSelection(); // unneeded
	
	/* The first new index will appear at insMinIndex and the last
	 * one will appear at insMaxIndex
	 */
        //NYI
        return; 
        
        /*
        
	long insMinIndex = (before) ? index : index + 1;
	long insMaxIndex = (insMinIndex + length) - 1;
	
	// shift everything after the insertion to the right
	SortedSet shiftSet = intervalSet.tailSet(new Index(insMinIndex));
	Set newSet = new TreeSet(); //FIXME: what type should this be optimially?
	Iterator i = shiftSet.iterator();
	while (i.hasNext()) {
	    BitSegment seg = (BitSegment)i.next();
	    BitSegment newSeg = new BitSegment(seg.firstIndex + length, seg.lastIndex + length);
	    newSet.add(newSeg);
	    
	    i.remove();
	}
	intervalSet.addAll(newSet);
	
	// shift part of the overhanging piece to the right
	BitSegment snipsnip = findLeftOverlap(new Index(insMinIndex), false);
	if (snipsnip != null) {
	    BitSegment rightOfSnip = new BitSegment(insMaxIndex + 1, snipsnip.lastIndex + length);
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
 */
    }
    
    private BitSegment moveSegment(BitSegment seg, long index, long length) {
        
        return null;
        //NYI. only used by insertIndexInterval(), above
        
        /*
	if (seg == EMPTY_SEGMENT)
	    return seg;
	
	boolean moveSeg = false;
	BitCursor movetoFirst = seg.firstIndex;;
	BitCursor movetoLast = seg.lastIndex;;
	
	if (seg.firstIndex >= index) {
	    movetoFirst = seg.firstIndex + length;
	    moveSeg = true;
	}
	
	if (seg.lastIndex >= index) {
	    movetoLast = seg.lastIndex + length;
	    moveSeg = true;
	}
	
	if (moveSeg) {
	    return new BitSegment(movetoFirst, movetoLast);
	}
	else {
	    return seg;
	}
	*/
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
     * Returns true if the bit to the right of the specified index is selected
     *
     */
    public boolean isSelectedIndex(BitCursor index) {
        /*
	if (index >= activeSelection.firstIndex
	    && index <= activeSelection.lastIndex) {
	    return true;
	}
         */
	
	TreeSet<BitSegment> tree = previewSaveActiveSelection();
	
	if (tree.isEmpty()) {
	    return false;
	}
	BitCursor indexPlus = index.addBits(1);
	SortedSet<BitSegment> headset = tree.headSet(new BitSegment(indexPlus,indexPlus)); 
	
	if (headset.isEmpty()) {
	    return false;
	}
	
	if ( headset.last().lastIndex.compareTo(index) < 0) {
	    return false;
	}
	
	return true;
	
    }
    
    /** Returns true if no indices are selected.
     *
     */
    public boolean isSelectionEmpty() {
	return previewSaveActiveSelection().isEmpty();
    }
    
    
    /** Set the anchor selection index.
     *
     * @see #getAnchorSelectionIndex
     *
     */
    public void setAnchor(BitCursor anchor) {
        // This also has the effect on saving the active selection and starting a new one.
	
        BitSegment change = new BitSegment(anchor, getAnchorSelectionIndex());
	
	if (selectionMode == MULTIPLE_INTERVAL_SELECTION) {
	    saveActiveSelection();
	    activeSelection = new DirectionalSegment(anchor, anchor);
	}
	else {
            clearSelection(); // shouldnt ?
            activeSelection = new DirectionalSegment(anchor, anchor);
            
            //fixme:
            
	    //angeActiveSelectionInterval(index, leadIndex, false);
	    //FIXME: clear or something?
	    //anchorIndex = index;
	}
	
	//isCacheValid = false; // handled by changeActiveSllection
	//isIntervalsetUptodate = false; // ditto
	
	fireValueChanged(change);
        
    }
    
    /** Set the lead selection index.
     *
     * @see #getLeadSelectionIndex
     *
     */
    public void setLeadSelectionIndex(BitCursor index) {
        BitCursor oldHead = getLeadSelectionIndex();
        
	changeActiveSelectionInterval(new DirectionalSegment(index, getAnchorSelectionIndex()), activeSelectionIsAnti);
        
	fireValueChanged(new BitSegment(index, oldHead));
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
	BitSegment oldActiveSelection = activeSelection;
	
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
	    BitSegment change = new BitSegment(getMinSelectionIndex(), getMaxSelectionIndex());
	    
	    intervalSet.clear(); // deliberately don't clear active selction
	    
	    isCacheValid = false;
	    isIntervalsetUptodate = false;
	    
	    fireValueChanged(change);
	    
	}
	else if (selectionMode == SINGLE_SELECTION) {
	    
	    BitSegment change = new BitSegment(getMinSelectionIndex(), getMaxSelectionIndex());
            //fixme: should check stuff.. but just dont use this mode, please.
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
	    BitSegment oldAdjustedSegment = adjustedSegment;
	    adjustedSegment = EMPTY_SEGMENT;
	    fireValueChanged( oldAdjustedSegment );
	    return;
	}
	
	this.isAdjusting = valueIsAdjusting;
    }
    
    /* can probably delete this. dont think it's used now */
    protected void fireValueChanged(BitCursor firstIndex, BitCursor lastIndex) {
        fireValueChanged(new BitSegment(firstIndex, lastIndex));
    }

        /**
     * @param firstIndex the first index in the interval
     * @param lastIndex the last index in the interval
     * @param isAdjusting true if this is the final change in a series of
     *		adjustments
     * @see EventListenerList
     */
    protected void fireValueChanged(BitSegment changedRange) {
	Object[] listeners = listenerList.getListenerList();
	boolean isAdjusting = getValueIsAdjusting();
	
	if (isAdjusting) {
            adjustedSegment = changedRange.maxRange(adjustedSegment);
	}
	else {
            adjustedSegment = EMPTY_SEGMENT;
	}
	
	BitSelectionEvent e = null;
	
	for (int i = listeners.length - 2; i >= 0; i -= 2) {
	    if (listeners[i] == BitSelectionListener.class) {
		if (e == null) {
		    e = new BitSelectionEvent(this, changedRange, isAdjusting);
		}
		((BitSelectionListener)listeners[i+1]).valueChanged(e);
	    }
	}
	
    }
    
    public Object clone() {
	try {
	    SegmentalBitSelectionModel clone = (SegmentalBitSelectionModel)super.clone();
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
            BitCursor size = getSelectionLength();
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
    /*
	public SegmentalBitSelectionModel slice(BitSegment slice) { // start, length replaced with BitSegment
                BitSegment[] bits = getSelectionIndexes();
		
		long index = start;
		long remain = length;
			
		if (start < 0) {
			// skip negatives
			index = 0;
			remain = length + start;
		}
		
		if (start >= bits.length)
			// start as after the end
			return new SegmentalBitSelectionModel(); // (-1,-1);
		
		SegmentalBitSelectionModel ret = new SegmentalBitSelectionModel();
		
		while (remain > 0 && index < bits.length) {
			ret.addSelectionInterval(bits[(int)index],bits[(int)index]);
			
			index++;
			remain--;
		}
		
		return ret;
	}
*/    
    
    
}
