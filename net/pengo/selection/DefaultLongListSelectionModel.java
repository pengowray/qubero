/*
 * @(#)DefaultListSelectionModel.java	1.65 01/12/03
 * @(#)DefaultLongListSelectionModel.java	1.00 2002-11-10
 *
 * Copyright 2002 Sun Microsystems, Inc. All rights reserved.
 * SUN PROPRIETARY/CONFIDENTIAL. Use is subject to license terms.
 *
 * Changed by Peter Halasz
 *
 * FIXME: Must all be rewritten. this class doesn't really work for longs properly because the
 * BitSet class does not use longs. (and it would suck if it did anyway).
 *
 * This class no longer used. Delete me.
 * DELME: Avoid copyright infringement.
 */

//package javax.swing;

package net.pengo.selection;

import java.util.EventListener;
import java.util.BitSet;
import java.io.Serializable;

import javax.swing.event.*;


/**
 * Default data model for list selections.
 * <p>
 * <strong>Warning:</strong>
 * Serialized objects of this class will not be compatible with
 * future Swing releases. The current serialization support is
 * appropriate for short term storage or RMI between applications running
 * the same version of Swing.  As of 1.4, support for long term storage
 * of all JavaBeans<sup><font size="-2">TM</font></sup>
 * has been added to the <code>java.beans</code> package.
 * Please see {@link java.beans.XMLEncoder}.
 *
 * @version 1.65 12/03/01
 * @author Philip Milne
 * @author Hans Muller
 * @see ListSelectionModel
 */

public class DefaultLongListSelectionModel implements LongListSelectionModel, Serializable
{
    private static final long MIN = -1;
    private static final long MAX = Long.MAX_VALUE;
    private int selectionMode = MULTIPLE_INTERVAL_SELECTION;
    private long minIndex = MAX;
    private long maxIndex = MIN;
    private long anchorIndex = -1;
    private long leadIndex = -1;
    private long firstAdjustedIndex = MAX;
    private long lastAdjustedIndex = MIN;
    private boolean isAdjusting = false;

    private long firstChangedIndex = MAX;
    private long lastChangedIndex = MIN;

    private BitSet value = new BitSet(32);
    protected EventListenerList listenerList = new EventListenerList();

    protected boolean leadAnchorNotificationEnabled = true;

    // implements javax.swing.ListSelectionModel
    public long getMinSelectionIndex() { return isSelectionEmpty() ? -1 : minIndex; }

    // implements javax.swing.ListSelectionModel
    public long getMaxSelectionIndex() { return maxIndex; }

    // implements javax.swing.ListSelectionModel
    public boolean getValueIsAdjusting() { return isAdjusting; }

    // implements javax.swing.ListSelectionModel
    /**
     * Returns the selection mode.
     * @return  one of the these values:
     * <ul>
     * <li>SINGLE_SELECTION
     * <li>SINGLE_INTERVAL_SELECTION
     * <li>MULTIPLE_INTERVAL_SELECTION
     * </ul>
     * @see #getSelectionMode
     */
    public int getSelectionMode() { return selectionMode; }

    // implements javax.swing.ListSelectionModel
    /**
     * Sets the selection mode.  The default is
     * MULTIPLE_INTERVAL_SELECTION.
     * @param selectionMode  one of three values:
     * <ul>
     * <li>SINGLE_SELECTION
     * <li>SINGLE_INTERVAL_SELECTION
     * <li>MULTIPLE_INTERVAL_SELECTION
     * </ul>
     * @exception IllegalArgumentException  if <code>selectionMode</code>
     *		is not one of the legal values shown above
     * @see #setSelectionMode
     */
    public void setSelectionMode(int selectionMode) {
	switch (selectionMode) {
	case SINGLE_SELECTION:
	case SINGLE_INTERVAL_SELECTION:
	case MULTIPLE_INTERVAL_SELECTION:
	    this.selectionMode = selectionMode;
	    break;
	default:
	    throw new IllegalArgumentException("invalid selectionMode");
	}
    }

    // implements javax.swing.ListSelectionModel
    public boolean isSelectedIndex(long index) {
	return ((index < minIndex) || (index > maxIndex)) ? false : value.get((int)(index)); //FIXME:!!
    }

    // implements javax.swing.ListSelectionModel
    public boolean isSelectionEmpty() {
	return (minIndex > maxIndex);
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
     * Returns an array of all the list selection listeners
     * registered on this <code>DefaultListSelectionModel</code>.
     *
     * @return all of this model's <code>ListSelectionListener</code>s
     *         or an empty
     *         array if no list selection listeners are currently registered
     *
     * @see #addListSelectionListener
     * @see #removeListSelectionListener
     *
     * @since 1.4
     */
    public ListSelectionListener[] getListSelectionListeners() {
        return (ListSelectionListener[])listenerList.getListeners(
                ListSelectionListener.class);
    }

    /**
     * Notifies listeners that we have ended a series of adjustments.
     */
    protected void fireValueChanged(boolean isAdjusting) {
        if (lastChangedIndex == MIN) {
	    return;
	}
	/* Change the values before sending the event to the
	 * listeners in case the event causes a listener to make
	 * another change to the selection.
	 */
	long oldFirstChangedIndex = firstChangedIndex;
	long oldLastChangedIndex = lastChangedIndex;
	firstChangedIndex = MAX;
	lastChangedIndex = MIN;
	fireValueChanged(oldFirstChangedIndex, oldLastChangedIndex, isAdjusting);
    }


    /**
     * Notifies <code>ListSelectionListeners</code> that the value
     * of the selection, in the closed interval <code>firstIndex</code>,
     * <code>lastIndex</code>, has changed.
     */
    protected void fireValueChanged(long firstIndex, long lastIndex) {
	fireValueChanged(firstIndex, lastIndex, getValueIsAdjusting());
    }

    /**
     * @param firstIndex the first index in the interval
     * @param lastIndex the last index in the interval
     * @param isAdjusting true if this is the final change in a series of
     *		adjustments
     * @see EventListenerList
     */
    protected void fireValueChanged(long firstIndex, long lastIndex, boolean isAdjusting)
    {
	Object[] listeners = listenerList.getListenerList();
        
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

    private void fireValueChanged() {
	if (lastAdjustedIndex == MIN) {
	    return;
	}
	/* If getValueAdjusting() is true, (eg. during a drag opereration)
	 * record the bounds of the changes so that, when the drag finishes (and
	 * setValueAdjusting(false) is called) we can post a single event
	 * with bounds covering all of these individual adjustments.
	 */
	if (getValueIsAdjusting()) {
            firstChangedIndex = Math.min(firstChangedIndex, firstAdjustedIndex);
	    lastChangedIndex = Math.max(lastChangedIndex, lastAdjustedIndex);
        }
	/* Change the values before sending the event to the
	 * listeners in case the event causes a listener to make
	 * another change to the selection.
	 */
	long oldFirstAdjustedIndex = firstAdjustedIndex;
	long oldLastAdjustedIndex = lastAdjustedIndex;
	firstAdjustedIndex = MAX;
	lastAdjustedIndex = MIN;

	fireValueChanged(oldFirstAdjustedIndex, oldLastAdjustedIndex);
    }

    /**
     * Returns an array of all the objects currently registered as
     * <code><em>Foo</em>Listener</code>s
     * upon this model.
     * <code><em>Foo</em>Listener</code>s
     * are registered using the <code>add<em>Foo</em>Listener</code> method.
     * <p>
     * You can specify the <code>listenerType</code> argument
     * with a class literal, such as <code><em>Foo</em>Listener.class</code>.
     * For example, you can query a <code>DefaultListSelectionModel</code>
     * instance <code>m</code>
     * for its list selection listeners
     * with the following code:
     *
     * <pre>ListSelectionListener[] lsls = (ListSelectionListener[])(m.getListeners(ListSelectionListener.class));</pre>
     *
     * If no such listeners exist,
     * this method returns an empty array.
     *
     * @param listenerType  the type of listeners requested;
     *          this parameter should specify an interface
     *          that descends from <code>java.util.EventListener</code>
     * @return an array of all objects registered as
     *          <code><em>Foo</em>Listener</code>s
     *          on this model,
     *          or an empty array if no such
     *          listeners have been added
     * @exception ClassCastException if <code>listenerType</code> doesn't
     *          specify a class or interface that implements
     *          <code>java.util.EventListener</code>
     *
     * @see #getListSelectionListeners
     *
     * @since 1.3
     */
    public EventListener[] getListeners(Class listenerType) {
	return listenerList.getListeners(listenerType);
    }

    // Updates first and last change indices
    private void markAsDirty(long r) {
        firstAdjustedIndex = Math.min(firstAdjustedIndex, r);
	lastAdjustedIndex =  Math.max(lastAdjustedIndex, r);
    }

    // Sets the state at this index and update all relevant state.
    private void set(long r) {
	if (value.get((int)r)) { //FIXME:
	    return;
	}
	value.set((int)r); //FIXME:!!
        markAsDirty(r);

        // Update minimum and maximum indices
        minIndex = Math.min(minIndex, r);
        maxIndex = Math.max(maxIndex, r);
    }

    // Clears the state at this index and update all relevant state.
    private void clear(long r) {
	if (!value.get((int)r)) { //FIXME:
	    return;
	}
	value.clear((int)r); //FIXME:
        markAsDirty(r);

        // Update minimum and maximum indices
        /*
           If (r > minIndex) the minimum has not changed.
           The case (r < minIndex) is not possible because r'th value was set.
           We only need to check for the case when lowest entry has been cleared,
           and in this case we need to search for the first value set above it.
	*/
	if (r == minIndex) {
	    for(minIndex = minIndex + 1; minIndex <= maxIndex; minIndex++) {
	        if (value.get((int)minIndex)) {
	            break;
	        }
	    }
	}
        /*
           If (r < maxIndex) the maximum has not changed.
           The case (r > maxIndex) is not possible because r'th value was set.
           We only need to check for the case when highest entry has been cleared,
           and in this case we need to search for the first value set below it.
	*/
	if (r == maxIndex) {
	    for(maxIndex = maxIndex - 1; minIndex <= maxIndex; maxIndex--) {
	        if (value.get((int)maxIndex)) {
	            break;
	        }
	    }
	}
	/* Performance note: This method is called from inside a loop in
	   changeSelection() but we will only iterate in the loops
	   above on the basis of one iteration per deselected cell - in total.
	   Ie. the next time this method is called the work of the previous
	   deselection will not be repeated.

	   We also don't need to worry about the case when the min and max
	   values are in their unassigned states. This cannot happen because
	   this method's initial check ensures that the selection was not empty
	   and therefore that the minIndex and maxIndex had 'real' values.

	   If we have cleared the whole selection, set the minIndex and maxIndex
	   to their cannonical values so that the next set command always works
	   just by using Math.min and Math.max.
	*/
        if (isSelectionEmpty()) {
            minIndex = MAX;
            maxIndex = MIN;
        }
    }

    /**
     * Sets the value of the leadAnchorNotificationEnabled flag.
     * @see		#isLeadAnchorNotificationEnabled()
     */
    public void setLeadAnchorNotificationEnabled(boolean flag) {
        leadAnchorNotificationEnabled = flag;
    }

    /**
     * Returns the value of the <code>leadAnchorNotificationEnabled</code> flag.
     * When <code>leadAnchorNotificationEnabled</code> is true the model
     * generates notification events with bounds that cover all the changes to
     * the selection plus the changes to the lead and anchor indices.
     * Setting the flag to false causes a narrowing of the event's bounds to
     * include only the elements that have been selected or deselected since
     * the last change. Either way, the model continues to maintain the lead
     * and anchor variables internally. The default is true.
     * @return 	the value of the <code>leadAnchorNotificationEnabled</code> flag
     * @see		#setLeadAnchorNotificationEnabled(boolean)
     */
    public boolean isLeadAnchorNotificationEnabled() {
        return leadAnchorNotificationEnabled;
    }

    private void updateLeadAnchorIndices(long anchorIndex, long leadIndex) {
        if (leadAnchorNotificationEnabled) {
            if (this.anchorIndex != anchorIndex) {
                if (this.anchorIndex != -1) { // The unassigned state.
                    markAsDirty(this.anchorIndex);
                }
                markAsDirty(anchorIndex);
            }

            if (this.leadIndex != leadIndex) {
                if (this.leadIndex != -1) { // The unassigned state.
                    markAsDirty(this.leadIndex);
                }
                markAsDirty(leadIndex);
            }
        }
        this.anchorIndex = anchorIndex;
        this.leadIndex = leadIndex;
    }

    private boolean contains(long a, long b, long i) {
        return (i >= a) && (i <= b);
    }

    private void changeSelection(long clearMin, long clearMax,
                                 long setMin, long setMax, boolean clearFirst) {
        for(long i = Math.min(setMin, clearMin); i <= Math.max(setMax, clearMax); i++) {

            boolean shouldClear = contains(clearMin, clearMax, i);
            boolean shouldSet = contains(setMin, setMax, i);

            if (shouldSet && shouldClear) {
                if (clearFirst) {
                    shouldClear = false;
                }
                else {
                    shouldSet = false;
                }
            }

            if (shouldSet) {
                set(i);
            }
            if (shouldClear) {
                clear(i);
            }
        }
        fireValueChanged();
    }

   /**   Change the selection with the effect of first clearing the values
    *   in the inclusive range [clearMin, clearMax] then setting the values
    *   in the inclusive range [setMin, setMax]. Do this in one pass so
    *   that no values are cleared if they would later be set.
    */
    private void changeSelection(long clearMin, long clearMax, long setMin, long setMax) {
        changeSelection(clearMin, clearMax, setMin, setMax, true);
    }

    // implements javax.swing.ListSelectionModel
    public void clearSelection() {
	removeSelectionInterval(minIndex, maxIndex);
    }

    // implements javax.swing.ListSelectionModel
    public void setSelectionInterval(long index0, long index1) {
        if (index0 == -1 || index1 == -1) {
            return;
        }

        if (getSelectionMode() == SINGLE_SELECTION) {
            index0 = index1;
        }

        updateLeadAnchorIndices(index0, index1);

        long clearMin = minIndex;
        long clearMax = maxIndex;
	long setMin = Math.min(index0, index1);
	long setMax = Math.max(index0, index1);
        changeSelection(clearMin, clearMax, setMin, setMax);
    }

    // implements javax.swing.ListSelectionModel
    public void addSelectionInterval(long index0, long index1)
    {
        if (index0 == -1 || index1 == -1) {
            return;
        }

        if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION) {
            setSelectionInterval(index0, index1);
            return;
        }

        updateLeadAnchorIndices(index0, index1);

        long clearMin = MAX;
        long clearMax = MIN;
	long setMin = Math.min(index0, index1);
	long setMax = Math.max(index0, index1);
        changeSelection(clearMin, clearMax, setMin, setMax);
    }


    // implements javax.swing.ListSelectionModel
    public void removeSelectionInterval(long index0, long index1)
    {
        if (index0 == -1 || index1 == -1) {
            return;
        }

        updateLeadAnchorIndices(index0, index1);

	long clearMin = Math.min(index0, index1);
	long clearMax = Math.max(index0, index1);
	long setMin = MAX;
	long setMax = MIN;

        // If the removal would produce to two disjoint selections in a mode
	// that only allows one, extend the removal to the end of the selection.
	if (getSelectionMode() != MULTIPLE_INTERVAL_SELECTION &&
	       clearMin > minIndex && clearMax < maxIndex) {
	    clearMax = maxIndex;
        }

        changeSelection(clearMin, clearMax, setMin, setMax);
    }

    private void setState(long index, boolean state) {
        if (state) {
	    set(index);
	}
	else {
	    clear(index);
	}
    }

    /**
     * Insert length indices beginning before/after index. If the value
     * at index is itself selected, set all of the newly inserted
     * items, otherwise leave them unselected. This method is typically
     * called to sync the selection model with a corresponding change
     * in the data model.
     */
    public void insertIndexInterval(long index, long length, boolean before)
    {
	/* The first new index will appear at insMinIndex and the last
	 * one will appear at insMaxIndex
	 */
	long insMinIndex = (before) ? index : index + 1;
	long insMaxIndex = (insMinIndex + length) - 1;

	/* Right shift the entire bitset by length, beginning with
	 * index-1 if before is true, index+1 if it's false (i.e. with
	 * insMinIndex).
	 */
	for(long i = maxIndex; i >= insMinIndex; i--) {
	    setState(i + length, value.get((int)i));
	}

	/* Initialize the newly inserted indices.
	 */
       	boolean setInsertedValues = value.get((int)index);
	for(long i = insMinIndex; i <= insMaxIndex; i++) {
	    setState(i, setInsertedValues);
	}
        fireValueChanged();
    }


    /**
     * Remove the indices in the interval index0,index1 (inclusive) from
     * the selection model.  This is typically called to sync the selection
     * model width a corresponding change in the data model.  Note
     * that (as always) index0 need not be <= index1.
     */
    public void removeIndexInterval(long index0, long index1)
    {
	long rmMinIndex = Math.min(index0, index1);
	long rmMaxIndex = Math.max(index0, index1);
	long gapLength = (rmMaxIndex - rmMinIndex) + 1;

	/* Shift the entire bitset to the left to close the index0, index1
	 * gap.
	 */
	for(long i = rmMinIndex; i <= maxIndex; i++) {
	    setState(i, value.get((int)(i + gapLength)));
	}
        fireValueChanged();
    }


    // implements javax.swing.ListSelectionModel
    public void setValueIsAdjusting(boolean isAdjusting) {
	if (isAdjusting != this.isAdjusting) {
	    this.isAdjusting = isAdjusting;
	    this.fireValueChanged(isAdjusting);
	}
    }


    /**
     * Returns a string that displays and identifies this
     * object's properties.
     *
     * @return a <code>String</code> representation of this object
     */
    public String toString() {
	String s =  ((getValueIsAdjusting()) ? "~" : "=") + value.toString();
	return getClass().getName() + " " + Integer.toString(hashCode()) + " " + s;
    }

    /**
     * Returns a clone of this selection model with the same selection.
     * <code>listenerLists</code> are not duplicated.
     *
     * @exception CloneNotSupportedException if the selection model does not
     *    both (a) implement the Cloneable interface and (b) define a
     *    <code>clone</code> method.
     */
    public Object clone() {
        try {
            DefaultLongListSelectionModel clone = (DefaultLongListSelectionModel)super.clone();
            clone.value = (BitSet)value.clone();
            clone.listenerList = new EventListenerList();
            return clone;
        } catch ( CloneNotSupportedException e) {
            //FIXME: hush hush
            e.printStackTrace();
            return null;
        }
    }

    // implements javax.swing.ListSelectionModel
    public long getAnchorSelectionIndex() {
        return anchorIndex;
    }

    // implements javax.swing.ListSelectionModel
    public long getLeadSelectionIndex() {
        return leadIndex;
    }

    /**
     * Set the anchor selection index, leaving all selection values unchanged.
     * If leadAnchorNotificationEnabled is true, send a notification covering
     * the old and new anchor cells.
     *
     * @see #getAnchorSelectionIndex
     * @see #setLeadSelectionIndex
     */
    public void setAnchorSelectionIndex(long anchorIndex) {
	updateLeadAnchorIndices(anchorIndex, this.leadIndex);
        this.anchorIndex = anchorIndex;
	fireValueChanged();
    }

    /**
     * Sets the lead selection index, ensuring that values between the
     * anchor and the new lead are either all selected or all deselected.
     * If the value at the anchor index is selected, first clear all the
     * values in the range [anchor, oldLeadIndex], then select all the values
     * values in the range [anchor, newLeadIndex], where oldLeadIndex is the old
     * leadIndex and newLeadIndex is the new one.
     * <p>
     * If the value at the anchor index is not selected, do the same thing in
     * reverse selecting values in the old range and deslecting values in the
     * new one.
     * <p>
     * Generate a single event for this change and notify all listeners.
     * For the purposes of generating minimal bounds in this event, do the
     * operation in a single pass; that way the first and last index inside the
     * [Long]ListSelectionEvent that is broadcast will refer to cells that actually
     * changed value because of this method. If, instead, this operation were
     * done in two steps the effect on the selection state would be the same
     * but two events would be generated and the bounds around the changed
     * values would be wider, including cells that had been first cleared only
     * to later be set.
     * <p>
     * This method can be used in the <code>mouseDragged</code> method
     * of a UI class to extend a selection.
     *
     * @see #getLeadSelectionIndex
     * @see #setAnchorSelectionIndex
     */
    public void setLeadSelectionIndex(long leadIndex) {
        long anchorIndex = this.anchorIndex;

	if ((anchorIndex == -1) || (leadIndex == -1)) {
	    return;
	}

	if (this.leadIndex == -1) {
	    this.leadIndex = leadIndex;
	}

	boolean shouldSelect = value.get((int)this.anchorIndex);

        if (getSelectionMode() == SINGLE_SELECTION) {
            anchorIndex = leadIndex;
            shouldSelect = true;
        }

        long oldMin = Math.min(this.anchorIndex, this.leadIndex);
        long oldMax = Math.max(this.anchorIndex, this.leadIndex);
	long newMin = Math.min(anchorIndex, leadIndex);
	long newMax = Math.max(anchorIndex, leadIndex);

	updateLeadAnchorIndices(anchorIndex, leadIndex);

        if (shouldSelect) {
            changeSelection(oldMin, oldMax, newMin, newMax);
        }
        else {
            changeSelection(newMin, newMax, oldMin, oldMax, false);
        }
    }
    
    //FIXME: this sucks but will be replaced when this class is rewritten
    public long[] getSelectionIndexes() {
        long[] indexes = new long[(int)(getMaxSelectionIndex() - getMinSelectionIndex())];
        int arraypos = 0;
        for (int last = 0; last != -1; last = value.nextSetBit(last)) {
            indexes[arraypos] = last;
            arraypos++;
        }
        
        long[] cpy = new long[arraypos];
        System.arraycopy(indexes, 0, cpy, 0, arraypos);
        
        return cpy;
    }
        
    
}

