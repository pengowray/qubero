/*
 * based on @(#)ListSelectionListener.java	1.10 01/12/03
 *  package javax.swing.event;
 *
 */

package net.pengo.selection;

import java.util.EventListener;


/** 
 * The listener that's notified when a lists selection value 
 * changes.
 */

public interface LongListSelectionListener extends EventListener
{
  void valueChanged(LongListSelectionEvent e);
}



