/*
 * AddressPage.java
 *
 * Created on July 12, 2003, 12:34 AM
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
public class AddressPage extends PropertyPage {
    private IntResource res;
    private IntResourcePropertiesForm form;
    private JTextField inputField;
    
    /** Creates a new instance of AddressPage */
    public AddressPage(IntResource res, IntResourcePropertiesForm form) {
        super();
        this.res = res;
        this.form = form;

        add(new JLabel( "Address: " ));
        inputField = new JTextField("0",12);
        build();
        add(inputField);
    }
    
    public void build() {
        inputField.setText(res.getSelectionData().getStart()+"");
    }
    
    public void save() {
    }
    
    public String toString() {
        return "Address";
    }
}
