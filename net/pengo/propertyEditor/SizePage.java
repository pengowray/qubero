/*
 * SizePage.java
 *
 * Created on July 11, 2003, 12:52 AM
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
public class SizePage extends PropertyPage {
    private IntResource res;
    private IntResourcePropertiesForm form;
    private JComboBox type;
    
    /** Creates a new instance of SizePage */
    public SizePage(IntResource res, IntResourcePropertiesForm form) {
        super();
        this.res = res;
        this.form = form;
        build();
    }
    
    public void save() {
        
    }

    public void build() {
        removeAll();
        add(new JLabel( "Size: " + res.getSelectionData().getLength() ));
    }
    
    
    public String toString() {
        return "Size";
    }
    
}
