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
import java.awt.event.*;

/**
 *
 * @author  Smiley
 */
public class SetTypePage extends PropertyPage {
    private IntResource res;
    private IntResourcePropertiesForm form;
    private JComboBox type;
    
    /** Creates a new instance of SummaryPage */
    public SetTypePage(IntResource res, IntResourcePropertiesForm form) {
        super();
        this.res = res;
        this.form = form;

        add(new JLabel( "Integer Type: " ));
        type = new JComboBox(new String[] {
            "unsigned", "ones complement (+/-0)", "twos complement (default)", "sign magnitude", "unused sign bit (NYI)"} );
        add(type);
        build(); // build before listener to stop apply being hit
        type.addActionListener( form.getModListener() );
    }
    
    public void build() {
        type.setSelectedIndex( res.getSigned() );
    }
    
    public void save() {
        res.setSigned( type.getSelectedIndex() );
        System.out.println("setting sign to: " + type.getSelectedIndex());
    }
    
    public String toString() {
        return "Type";
    }
}
