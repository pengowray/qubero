/*
 * SummaryPage.java
 *
 * Created on July 9, 2003, 11:26 PM
 */

package net.pengo.propertyEditor;
import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

/**
 *
 * @author  Smiley
 */
public class SummaryPage extends PropertyPage {
    private IntResource res;
    /** Creates a new instance of SummaryPage */
    public SummaryPage(IntResource res) {
        super();
        this.res = res;
        build();
    }
    
    public void build() {
        removeAll();
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        //add(new JLabel( "Size (bytes): " + res.getSelectionData().getLength() ));
        
        //add(new JLabel( "Value: " + res.getValue().toString() ));
        JTextPane jtp = new JTextPane();
        String s = "Value: " + res.getValue() + "\n";
        s = s  + "Size (bytes): " + res.getSelectionResource().getSelectionData().getLength();
        jtp.setText(s ) ;
        add(jtp);
    }
    
    public void save() {
        return;
    }
    
    public String toString() {
        return "Summary";
    }
}
